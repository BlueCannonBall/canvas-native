//! In-process worker for Canvas. No connector executable or MCP transport.
use crate::engine::Engine;
use anyhow::{Context, Result, bail};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{path::PathBuf, time::Duration};
use tokio::sync::{mpsc, oneshot, watch};

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Settings {
    pub origin: String,
    #[serde(default)]
    pub appearance: crate::appearance::Appearance,
}
impl Settings {
    fn path() -> Result<PathBuf> {
        Ok(ProjectDirs::from("", "", "canvas-native")
            .context("No configuration directory")?
            .config_dir()
            .join("settings.json"))
    }
    pub fn load() -> Self {
        let mut settings: Self = Self::path()
            .ok()
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        if let Ok(origin) = std::env::var("CANVAS_URL") {
            settings.origin = origin;
        }
        settings
    }
    pub fn validate(&self) -> Result<()> {
        let url = url::Url::parse(self.origin.trim())
            .context("Enter your university’s Canvas website address.")?;
        if !matches!(url.scheme(), "https" | "http")
            || url.host_str().is_none()
            || url.path() != "/"
            || url.query().is_some()
            || url.fragment().is_some()
            || !url.username().is_empty()
            || url.password().is_some()
        {
            bail!("Use your Canvas website address, without a page path or embedded credentials.");
        }
        Ok(())
    }
    fn save(&self) -> Result<()> {
        self.validate()?;
        self.save_preferences()
    }
    fn save_preferences(&self) -> Result<()> {
        let path = Self::path()?;
        std::fs::create_dir_all(path.parent().context("Missing configuration directory")?)?;
        std::fs::write(path, serde_json::to_vec_pretty(self)?)?;
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub enum Operation {
    Discover,
    Configure(Settings),
    Tool { name: String, args: Value },
    Resource(String),
    Appearance(crate::appearance::Appearance),
}
struct Job {
    operation: Operation,
    reply: oneshot::Sender<Result<Value>>,
    generation: u64,
}
pub struct Backend {
    jobs: mpsc::UnboundedSender<Job>,
    cancel: watch::Sender<u64>,
}
impl Clone for Backend {
    fn clone(&self) -> Self {
        Self {
            jobs: self.jobs.clone(),
            cancel: self.cancel.clone(),
        }
    }
}
impl Backend {
    pub fn start(settings: Settings, runtime: &tokio::runtime::Handle) -> Self {
        let (jobs, mut receiver) = mpsc::unbounded_channel::<Job>();
        let (cancel, mut cancellation) = watch::channel(0);
        runtime.spawn(async move {
            let mut settings = settings;
            let mut engine:Option<Engine> = None;
            let mut reads=tokio::task::JoinSet::new();
            while let Some(job) = receiver.recv().await {
                while reads.try_join_next().is_some() {}
                if *cancellation.borrow_and_update() != job.generation {
                    let _ = job.reply.send(Err(anyhow::anyhow!("Cancelled before starting."))); continue;
                }
                if matches!(&job.operation,Operation::Tool{name,..} if is_cacheable_read(name)) {
                    while reads.len()>=4 { let _=reads.join_next().await; }
                    let ready=settings.validate().and_then(|()|{
                        if engine.is_none(){engine=Some(Engine::new(settings.origin.trim())?);}
                        Ok(engine.as_ref().expect("initialized engine").clone())
                    });
                    match ready {
                        Ok(engine)=>{let cancel=cancellation.clone();reads.spawn(run_read(job,engine,cancel));},
                        Err(error)=>{let _=job.reply.send(Err(error));},
                    }
                    continue;
                }
                // Authentication/configuration and writes remain barriers. Only
                // known read-only calls can execute concurrently or be prefetched.
                while reads.join_next().await.is_some() {}
                if *cancellation.borrow_and_update()!=job.generation {
                    let _=job.reply.send(Err(anyhow::anyhow!("Cancelled before starting.")));continue;
                }
                let timeout = if matches!(&job.operation, Operation::Tool { name, .. } if name == "auth") { 900 } else { 180 };
                let result = tokio::select! {
                    _ = cancellation.changed() => Err(anyhow::anyhow!("Cancelled. A change already accepted by Canvas cannot be undone; check your to-do list before retrying.")),
                    result = tokio::time::timeout(Duration::from_secs(timeout), perform(job.operation, &mut settings, &mut engine)) =>
                        result.unwrap_or_else(|_| Err(anyhow::anyhow!("Canvas took too long to respond. Changes are never retried automatically."))),
                };
                let _ = job.reply.send(result);
            }
            while reads.join_next().await.is_some() {}
        });
        Self { jobs, cancel }
    }
    pub fn request(&self, operation: Operation) -> oneshot::Receiver<Result<Value>> {
        let (reply, receiver) = oneshot::channel();
        let _ = self.jobs.send(Job {
            operation,
            reply,
            generation: *self.cancel.borrow(),
        });
        receiver
    }
    pub fn cancel(&self) {
        self.cancel.send_modify(|v| *v = v.wrapping_add(1));
    }
}

pub fn is_cacheable_read(name: &str) -> bool {
    if name == "course_front_page" {
        return true;
    }
    matches!(
        name,
        "course_list"
            | "course_info"
            | "module_list"
            | "assignment_list"
            | "assignment_info"
            | "assignment_grade"
            | "course_grades"
            | "grade_summary"
            | "dashboard_items"
            | "page_info"
            | "file_info"
            | "planner_note_info"
            | "attachment_text"
            | "attachment_image"
            | "document_preview"
    )
}
async fn run_read(job: Job, engine: Engine, mut cancellation: watch::Receiver<u64>) {
    if *cancellation.borrow_and_update() != job.generation {
        let _ = job
            .reply
            .send(Err(anyhow::anyhow!("Cancelled before starting.")));
        return;
    }
    let Operation::Tool { name, args } = job.operation else {
        unreachable!()
    };
    let result = tokio::select! {
        _=cancellation.changed()=>Err(anyhow::anyhow!("Read cancelled.")),
        result=tokio::time::timeout(Duration::from_secs(180),engine.call(&name,args))=>result.unwrap_or_else(|_|Err(anyhow::anyhow!("Canvas took too long to respond."))),
    };
    let _ = job.reply.send(result);
}
impl Drop for Backend {
    fn drop(&mut self) {
        self.cancel();
    }
}

async fn perform(
    operation: Operation,
    settings: &mut Settings,
    engine: &mut Option<Engine>,
) -> Result<Value> {
    match operation {
        Operation::Appearance(appearance) => {
            settings.appearance = appearance;
            settings.save_preferences()?;
            Ok(json!({"preferences_saved":true}))
        }
        Operation::Discover => Engine::operation_catalog(),
        Operation::Configure(new) => {
            new.save()?;
            *engine = None;
            *settings = new;
            Ok(json!({"saved":true}))
        }
        operation => {
            settings.validate()?;
            if engine.is_none() {
                *engine = Some(Engine::new(settings.origin.trim())?);
            }
            let engine = engine.as_ref().context("Canvas is not configured")?;
            match operation {
                Operation::Tool { name, args } => engine.call(&name, args).await,
                Operation::Resource(uri) => engine.resource(uri).await,
                _ => unreachable!(),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_known_read_only_actions_can_run_concurrently() {
        for name in [
            "auth",
            "refresh_auth",
            "create_planner_note",
            "update_custom_planner_item",
            "delete_planner_note",
            "download_attachment",
            "unknown_action",
        ] {
            assert!(!is_cacheable_read(name));
        }
        for name in [
            "module_list",
            "course_info",
            "assignment_list",
            "file_info",
            "document_preview",
        ] {
            assert!(is_cacheable_read(name));
        }
    }
    #[test]
    fn rejects_credentials_and_page_addresses() {
        for origin in [
            "https://user:secret@canvas.test",
            "https://canvas.test/courses/1",
            "https://canvas.test/?token=x",
            "file:///etc/passwd",
        ] {
            assert!(
                Settings {
                    origin: origin.into(),
                    ..Default::default()
                }
                .validate()
                .is_err()
            );
        }
        assert!(
            Settings {
                origin: "https://canvas.test".into(),
                ..Default::default()
            }
            .validate()
            .is_ok()
        );
    }
    #[test]
    fn existing_settings_default_to_system_appearance() {
        let settings: Settings =
            serde_json::from_str("{\"origin\":\"https://canvas.test\"}").unwrap();
        assert_eq!(settings.appearance, crate::appearance::Appearance::System);
        assert!(settings.validate().is_ok());
        let mut settings = settings;
        settings.appearance = crate::appearance::Appearance::Dark;
        let restored: Settings =
            serde_json::from_value(serde_json::to_value(settings).unwrap()).unwrap();
        assert_eq!(restored.appearance, crate::appearance::Appearance::Dark);
    }
    #[tokio::test]
    async fn discovers_all_operations_without_configuration_or_executable() {
        let worker = Backend::start(Settings::default(), &tokio::runtime::Handle::current());
        let tools = worker.request(Operation::Discover).await.unwrap().unwrap();
        assert_eq!(tools.as_array().unwrap().len(), 24);
    }
    #[tokio::test]
    async fn cancellation_prevents_queued_operations() {
        let worker = Backend::start(Settings::default(), &tokio::runtime::Handle::current());
        let result = worker.request(Operation::Tool {
            name: "create_planner_note".into(),
            args: json!({"title":"must not run"}),
        });
        worker.cancel();
        assert!(
            result
                .await
                .unwrap()
                .unwrap_err()
                .to_string()
                .contains("Cancelled before")
        );
    }
}
