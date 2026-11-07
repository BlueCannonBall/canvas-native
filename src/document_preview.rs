//! Session-private document files and bounded, on-demand PDF rasterization.
//! Rendering is pure Rust (hayro), so previews need no system PDF tools and
//! behave the same on Linux, macOS and Windows.
use anyhow::{Context, Result, bail, ensure};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use hayro::hayro_interpret::InterpreterSettings;
use hayro::hayro_syntax::Pdf;
use hayro::vello_cpu::color::palette::css::WHITE;
use hayro::{PixmapSettings, RenderCache, RenderSettings};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    future::Future,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};
use tempfile::TempDir;
use tokio::sync::{Mutex, OnceCell};

pub const MAX_DOCUMENT_BYTES: usize = 64 * 1024 * 1024;
pub const DEFAULT_RASTER_SIZE: u32 = 1800;
type RenderCell = Arc<OnceCell<PathBuf>>;
type DocumentCell = Arc<OnceCell<Arc<StoredDocument>>>;
type ClassifiedDocument = (String, String, &'static str, u32, Vec<u8>);
pub struct DocumentBytes {
    pub bytes: Vec<u8>,
    pub mime_type: Option<String>,
    pub filename: Option<String>,
}
struct StoredDocument {
    _directory: Arc<TempDir>,
    path: PathBuf,
    filename: String,
    mime_type: String,
    kind: &'static str,
    pages: OnceCell<u32>,
    renders: Mutex<HashMap<(u32, u32), RenderCell>>,
}
#[derive(Default)]
pub struct PreviewStore {
    directory: Mutex<Option<Arc<TempDir>>>,
    next_id: AtomicU64,
    documents: Mutex<HashMap<String, DocumentCell>>,
}
impl PreviewStore {
    async fn directory(&self) -> Result<Arc<TempDir>> {
        let mut directory = self.directory.lock().await;
        if directory.is_none() {
            let base = directories::ProjectDirs::from("", "", "canvas-native")
                .context("Cannot find document cache directory")?
                .cache_dir()
                .to_path_buf();
            tokio::fs::create_dir_all(&base).await?;
            *directory = Some(Arc::new(
                tempfile::Builder::new()
                    .prefix("preview-session-")
                    .tempdir_in(base)?,
            ));
        }
        Ok(directory
            .as_ref()
            .expect("initialized document directory")
            .clone())
    }
    pub async fn clear(&self) {
        self.documents.lock().await.clear();
        // Only our freshly-created private TempDir, never Downloads or
        // user-selected files. Arc guards protect any active reader.
        self.directory.lock().await.take();
    }
    pub async fn cached_source(&self, resource: &str) -> Result<Option<DocumentBytes>> {
        let cell = self
            .documents
            .lock()
            .await
            .get(&resource_key(resource))
            .cloned();
        let Some(document) = cell.as_ref().and_then(|cell| cell.get()) else {
            return Ok(None);
        };
        Ok(Some(DocumentBytes {
            bytes: tokio::fs::read(&document.path).await?,
            mime_type: Some(document.mime_type.clone()),
            filename: Some(document.filename.clone()),
        }))
    }
    pub async fn preview<F, Fut>(
        &self,
        resource: &str,
        page: u32,
        raster_size: u32,
        load: F,
    ) -> Result<Value>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<DocumentBytes>>,
    {
        ensure!(page > 0, "Page numbers start at 1.");
        ensure!(
            (400..=2800).contains(&raster_size),
            "Preview size must be between 400 and 2800 pixels."
        );
        let key = resource_key(resource);
        let cell = self.documents.lock().await.entry(key).or_default().clone();
        let document=cell.get_or_try_init(||async{
            let source=load().await?;
            ensure!(source.bytes.len()<=MAX_DOCUMENT_BYTES,"This document is too large for an in-app preview. Download it to view externally.");
            let directory=self.directory().await?;
            let path=directory.path().join(format!("document-{}.bin",self.next_id.fetch_add(1,Ordering::Relaxed)));
            let(filename,mime_type,kind,pages,bytes)=tokio::task::spawn_blocking(move||classify(source)).await??;
            tokio::fs::write(&path,bytes.as_slice()).await?;
            #[cfg(unix)] {use std::os::unix::fs::PermissionsExt;tokio::fs::set_permissions(&path,std::fs::Permissions::from_mode(0o600)).await?;}
            Ok::<_,anyhow::Error>(Arc::new(StoredDocument{_directory:directory,path,filename,mime_type,kind,pages:OnceCell::new_with((pages>0).then_some(pages)),renders:Mutex::new(HashMap::new())}))
        }).await?;
        // Source caching is independent of the renderer. Even if a PDF is
        // damaged or encrypted, Text & images can reuse the download.
        let pages = *document
            .pages
            .get_or_try_init(|| {
                let path = document.path.clone();
                async move { tokio::task::spawn_blocking(move || pdf_page_count(&path)).await? }
            })
            .await?;
        ensure!(page <= pages, "This document has {} pages.", pages);
        let mut metadata = json!({"resource":resource,"filename":document.filename,"mime_type":document.mime_type,"kind":document.kind,"pages":pages,"page":page,"raster_size":raster_size});
        match document.kind {
            "pdf" => {
                let cell = document
                    .renders
                    .lock()
                    .await
                    .entry((page, raster_size))
                    .or_default()
                    .clone();
                let source = document.path.clone();
                let image = document
                    .path
                    .with_extension(format!("page-{page}-{raster_size}.png"));
                let output = image.clone();
                let image = cell
                    .get_or_try_init(move || async move {
                        tokio::task::spawn_blocking(move || {
                            render_pdf_page(&source, page, raster_size, &output)
                        })
                        .await??;
                        let size = tokio::fs::metadata(&image).await?.len();
                        ensure!(
                            size <= 40 * 1024 * 1024,
                            "Rendered page exceeded the preview size limit."
                        );
                        Ok::<_, anyhow::Error>(image)
                    })
                    .await?;
                metadata["image_path"] = json!(image);
                metadata["image_mime_type"] = json!("image/png");
            }
            "image" => {
                metadata["image_path"] = json!(document.path);
                metadata["image_mime_type"] = json!(document.mime_type);
            }
            _ => {}
        }
        // Page aspect ratios let the scrolling reader lay out every page before
        // its raster arrives. SVG keeps its declarative size instead.
        let dimensions = match (
            metadata["image_path"].as_str().map(PathBuf::from),
            metadata["image_mime_type"].as_str(),
        ) {
            (Some(path), Some(mime)) if mime.starts_with("image/") && mime != "image/svg+xml" => {
                tokio::task::spawn_blocking(move || image::image_dimensions(&path))
                    .await
                    .ok()
                    .and_then(|result| result.ok())
            }
            _ => None,
        };
        if let Some((width, height)) = dimensions {
            ensure!(
                width > 0 && height > 0,
                "Document image has invalid dimensions."
            );
            metadata["image_width"] = json!(width);
            metadata["image_height"] = json!(height);
        }
        Ok(json!({"structuredContent":{"document":metadata}}))
    }
}
fn resource_key(resource: &str) -> String {
    resource
        .strip_prefix("canvas-text://")
        .map(|s| format!("canvas://{s}"))
        .unwrap_or_else(|| resource.to_owned())
}
fn pdf_error(error: hayro::hayro_syntax::LoadPdfError) -> anyhow::Error {
    use hayro::hayro_syntax::LoadPdfError;
    match error {
        LoadPdfError::Decryption(_) => {
            anyhow::anyhow!("This PDF is password protected. Download it to open it externally.")
        }
        LoadPdfError::Invalid => anyhow::anyhow!(
            "This PDF could not be read. It may be damaged. Text & images and Download remain available."
        ),
    }
}
fn pdf_page_count(path: &Path) -> Result<u32> {
    let bytes = std::fs::read(path)?;
    let pdf = Pdf::new(bytes).map_err(pdf_error)?;
    let pages = u32::try_from(pdf.pages().len()).context("This PDF has too many pages.")?;
    ensure!(pages > 0, "This PDF has no readable pages.");
    Ok(pages)
}
fn render_pdf_page(path: &Path, page: u32, raster_size: u32, output: &Path) -> Result<()> {
    let bytes = std::fs::read(path)?;
    let pdf = Pdf::new(bytes).map_err(pdf_error)?;
    let pages = pdf.pages();
    let index = usize::try_from(page - 1).context("PDF page number is out of range.")?;
    let page = pages
        .get(index)
        .context("This PDF has fewer pages than the reader requested.")?;
    let (width, height) = page.render_dimensions();
    ensure!(
        width.is_finite() && height.is_finite() && width > 0.0 && height > 0.0,
        "This PDF page has no printable area."
    );
    let scale = raster_size as f32 / width;
    let pixel_height = (height * scale).ceil();
    ensure!(
        pixel_height > 0.0
            && pixel_height <= 8000.0
            && u64::from(raster_size) * pixel_height as u64 <= 40_000_000,
        "This PDF page is too large to render at this zoom. Try a smaller zoom."
    );
    let pixmap = hayro::render(
        page,
        &RenderCache::new(),
        &InterpreterSettings::default(),
        &RenderSettings::default(),
        &PixmapSettings {
            x_scale: scale,
            y_scale: scale,
            bg_color: WHITE,
        },
    );
    let png = pixmap
        .into_png()
        .context("The rendered PDF page could not be encoded.")?;
    std::fs::write(output, png)?;
    Ok(())
}
fn classify(source: DocumentBytes) -> Result<ClassifiedDocument> {
    let mut mime = source
        .mime_type
        .as_deref()
        .unwrap_or("")
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_lowercase();
    let filename = source.filename.unwrap_or_else(|| "Document".into());
    if mime.is_empty() || mime == "application/octet-stream" {
        let name = filename.to_lowercase();
        mime = if source.bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
            "image/png"
        } else if source.bytes.starts_with(b"\xff\xd8\xff") {
            "image/jpeg"
        } else if name.ends_with(".docx") {
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
        } else if name.ends_with(".txt") || name.ends_with(".md") || name.ends_with(".csv") {
            "text/plain"
        } else if name.ends_with(".json") {
            "application/json"
        } else if name.ends_with(".xml") {
            "application/xml"
        } else {
            mime.as_str()
        }
        .to_owned();
    }
    let (mime, kind, pages) = if source.bytes.starts_with(b"%PDF-") || mime == "application/pdf" {
        ("application/pdf".into(), "pdf", 0)
    } else if matches!(
        mime.as_str(),
        "image/png"
            | "image/jpeg"
            | "image/jpg"
            | "image/webp"
            | "image/gif"
            | "image/svg+xml"
            | "image/bmp"
            | "image/tiff"
            | "image/tif"
    ) {
        let (width, height) = if mime == "image/svg+xml" {
            ensure!(
                source.bytes.len() <= 2 * 1024 * 1024,
                "This SVG is too large for an in-app preview."
            );
            validate_svg(&source.bytes)?;
            let mut options = usvg::Options::default();
            options.image_href_resolver.resolve_string = Box::new(|_, _| None);
            let tree = usvg::Tree::from_data(&source.bytes, &options)
                .context("Cannot read this SVG image")?;
            (
                tree.size().width().ceil() as u32,
                tree.size().height().ceil() as u32,
            )
        } else {
            image::ImageReader::new(std::io::Cursor::new(&source.bytes))
                .with_guessed_format()?
                .into_dimensions()
                .context("Cannot read image dimensions")?
        };
        ensure!(
            width > 0
                && height > 0
                && width <= 12000
                && height <= 12000
                && (width as u64) * (height as u64) <= 32_000_000,
            "This image is too large for an in-app preview. Download it to view externally."
        );
        (mime, "image", 1)
    } else {
        (mime, "text", 1)
    };
    Ok((filename, mime, kind, pages, source.bytes))
}

/// An LMS SVG must never resolve paths on the viewer's local machine. The
/// native image loader accepts SVG bytes, so validate resource references before
/// either dimension parsing or display. Keep the original artwork unchanged.
pub fn validate_svg(bytes: &[u8]) -> Result<()> {
    ensure!(
        bytes.len() <= 2 * 1024 * 1024,
        "SVG exceeds the preview size limit."
    );
    ensure!(
        !bytes.starts_with(b"\x1f\x8b"),
        "Compressed SVG previews are not supported; download the original."
    );
    std::str::from_utf8(bytes).context("SVG preview requires UTF-8 XML")?;
    let mut reader = quick_xml::Reader::from_reader(bytes);
    let mut root_seen = false;
    let mut embedded_pixels = 0u64;
    loop {
        match reader.read_event()? {
            quick_xml::events::Event::Start(tag) | quick_xml::events::Event::Empty(tag) => {
                if !root_seen {
                    ensure!(tag.local_name().as_ref() == b"svg", "Not an SVG document");
                    root_seen = true;
                }
                if !matches!(
                    tag.local_name().as_ref(),
                    b"image" | b"feImage" | b"use" | b"color-profile" | b"font-face-uri"
                ) {
                    continue;
                }
                for attribute in tag.attributes() {
                    let attribute = attribute?;
                    if attribute.key.local_name().as_ref() != b"href" {
                        continue;
                    }
                    let reference = attribute.decode_and_unescape_value(reader.decoder())?;
                    // Only <use> resolves fragments structurally. The image
                    // resolver otherwise treats even '#name' as a local path.
                    if reference.starts_with('#') && tag.local_name().as_ref() == b"use" {
                        continue;
                    }
                    let Some((header, encoded)) = reference.split_once(',') else {
                        bail!(
                            "SVGs with external image/file references must be downloaded for viewing."
                        );
                    };
                    ensure!(
                        header.contains(";base64")
                            && matches!(
                                header.split(';').next(),
                                Some(
                                    "data:image/png"
                                        | "data:image/jpeg"
                                        | "data:image/jpg"
                                        | "data:image/gif"
                                        | "data:image/webp"
                                        | "data:image/bmp"
                                )
                            ),
                        "Unsupported SVG image reference. Download the original to view externally."
                    );
                    let image = STANDARD.decode(encoded)?;
                    let (width, height) = image::ImageReader::new(std::io::Cursor::new(image))
                        .with_guessed_format()?
                        .into_dimensions()?;
                    embedded_pixels =
                        embedded_pixels.saturating_add((width as u64) * (height as u64));
                    ensure!(
                        width > 0 && height > 0 && embedded_pixels <= 32_000_000,
                        "Embedded SVG images exceed the preview limit."
                    );
                }
            }
            quick_xml::events::Event::DocType(_) => {
                bail!("SVG document type declarations are not allowed in a preview.")
            }
            quick_xml::events::Event::Eof => break,
            _ => {}
        }
    }
    ensure!(root_seen, "Not an SVG document");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn svg_preview_rejects_local_and_remote_resource_reads() {
        for href in [
            "file:///home/user/private.png",
            "/home/user/private.png",
            "relative.png",
            "#private.png",
            "https://example.test/image.png",
            "data:image/svg+xml;base64,PHN2Zy8+",
        ] {
            let svg =
                format!("<svg xmlns=\"http://www.w3.org/2000/svg\"><image href=\"{href}\"/></svg>");
            assert!(validate_svg(svg.as_bytes()).is_err());
        }
        assert!(
            validate_svg(
                br##"<svg xmlns="http://www.w3.org/2000/svg"><use href="#shape"/></svg>"##
            )
            .is_ok()
        );
    }
    #[test]
    fn svg_preview_rejects_external_entity_declarations() {
        assert!(validate_svg(br#"<!DOCTYPE svg SYSTEM "file:///private.dtd"><svg/>"#).is_err());
    }
    #[test]
    fn svg_validation_cannot_be_bypassed_with_compressed_or_non_xml_bytes() {
        assert!(validate_svg(b"\x1f\x8b compressed").is_err());
        assert!(validate_svg(b"not xml").is_err());
        assert!(validate_svg(b"<not-svg/>").is_err());
    }
    #[test]
    fn resource_aliases_share_the_download_cache() {
        assert_eq!(
            resource_key("canvas-text://files/4/download"),
            resource_key("canvas://files/4/download")
        );
    }
    #[tokio::test]
    async fn rejects_invalid_page_and_render_size_before_fetching() {
        let store = PreviewStore::default();
        for (page, size) in [(0, 1800), (1, 10), (1, 9000)] {
            assert!(
                store
                    .preview("canvas://files/1/download", page, size, || async {
                        panic!("must not fetch")
                    })
                    .await
                    .is_err()
            );
        }
    }
    #[tokio::test]
    async fn renderer_failure_does_not_discard_the_source_download() {
        let store = PreviewStore::default();
        *store.directory.lock().await = Some(Arc::new(
            tempfile::Builder::new()
                .prefix("canvas-pdf-error-test-")
                .tempdir()
                .unwrap(),
        ));
        let loads = std::sync::atomic::AtomicUsize::new(0);
        for _ in 0..2 {
            assert!(
                store
                    .preview("canvas://files/2/download", 1, 600, || async {
                        loads.fetch_add(1, Ordering::SeqCst);
                        Ok(DocumentBytes {
                            bytes: b"%PDF-1.4\nbroken".to_vec(),
                            mime_type: Some("application/pdf".into()),
                            filename: Some("Broken.pdf".into()),
                        })
                    })
                    .await
                    .is_err()
            );
        }
        assert_eq!(loads.load(Ordering::SeqCst), 1);
        assert_eq!(
            store
                .cached_source("canvas://files/2/download")
                .await
                .unwrap()
                .unwrap()
                .bytes,
            b"%PDF-1.4\nbroken"
        );
    }
    fn pdf() -> Vec<u8> {
        use lopdf::{Document, Object, Stream, dictionary};
        let mut doc = Document::with_version("1.5");
        let pages_id = doc.new_object_id();
        let mut kids = Vec::new();
        for color in ["0 0 1", "1 0 0"] {
            let content = doc.add_object(Stream::new(
                dictionary! {},
                format!("{color} rg 20 20 160 100 re f").into_bytes(),
            ));
            let page=doc.add_object(dictionary!{"Type"=>"Page","Parent"=>pages_id,"MediaBox"=>vec![0.into(),0.into(),200.into(),140.into()],"Contents"=>content});
            kids.push(Object::Reference(page));
        }
        doc.objects.insert(
            pages_id,
            dictionary! {"Type"=>"Pages","Kids"=>kids,"Count"=>2}.into(),
        );
        let catalog = doc.add_object(dictionary! {"Type"=>"Catalog","Pages"=>pages_id});
        doc.trailer.set("Root", catalog);
        let mut bytes = Vec::new();
        doc.save_to(&mut bytes).unwrap();
        bytes
    }
    #[tokio::test]
    async fn renders_pdf_pages_and_reuses_download_and_raster_cache() {
        let store = PreviewStore::default();
        *store.directory.lock().await = Some(Arc::new(
            tempfile::Builder::new()
                .prefix("canvas-pdf-test-")
                .tempdir()
                .unwrap(),
        ));
        let count = std::sync::atomic::AtomicUsize::new(0);
        let source = || async {
            count.fetch_add(1, Ordering::SeqCst);
            Ok(DocumentBytes {
                bytes: pdf(),
                mime_type: Some("application/pdf".into()),
                filename: Some("Test.pdf".into()),
            })
        };
        let a = store
            .preview("canvas://files/1/download", 1, 600, source)
            .await
            .unwrap();
        let b = store
            .preview("canvas-text://files/1/download", 1, 600, source)
            .await
            .unwrap();
        let c = store
            .preview("canvas://files/1/download", 2, 600, source)
            .await
            .unwrap();
        assert_eq!(count.load(Ordering::SeqCst), 1);
        assert_eq!(
            a["structuredContent"]["document"]["image_path"],
            b["structuredContent"]["document"]["image_path"]
        );
        assert_eq!(a["structuredContent"]["document"]["pages"], 2);
        let image = |value: &Value| {
            std::fs::read(
                value["structuredContent"]["document"]["image_path"]
                    .as_str()
                    .unwrap(),
            )
            .unwrap()
        };
        assert_ne!(image(&a), image(&c));
        let png = image(&a);
        let info = png::Decoder::new(std::io::Cursor::new(png))
            .read_info()
            .unwrap();
        assert_eq!(info.info().width, 600);
        assert!(info.info().height <= 600);
        fn center(value: Vec<u8>) -> [u8; 3] {
            let mut reader = png::Decoder::new(std::io::Cursor::new(value))
                .read_info()
                .unwrap();
            let mut pixels = vec![0; reader.output_buffer_size()];
            let info = reader.next_frame(&mut pixels).unwrap();
            let channels = info.color_type.samples();
            let index = ((info.height / 2 * info.width + info.width / 2) as usize) * channels;
            [pixels[index], pixels[index + 1], pixels[index + 2]]
        }
        assert_eq!(center(image(&a)), [0, 0, 255]);
        assert_eq!(center(image(&c)), [255, 0, 0]);
        assert!(
            store
                .cached_source("canvas://files/1/download")
                .await
                .unwrap()
                .unwrap()
                .bytes
                .starts_with(b"%PDF-")
        );
        let path = PathBuf::from(
            a["structuredContent"]["document"]["image_path"]
                .as_str()
                .unwrap(),
        );
        store.clear().await;
        assert!(!path.exists());
    }
}
