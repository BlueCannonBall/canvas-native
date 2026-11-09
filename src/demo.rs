//! Isolated sample account. Never a fallback for failed real requests.
use crate::{backend::Operation, model};
use anyhow::{Result, bail};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use chrono::Local;
use serde_json::{Value, json};

/// Deterministic named starting screens for screenshot/regression checks. This
/// override is consulted only in the explicitly selected sample account.
pub fn initial_route() -> model::Route {
    use model::{CourseTab, Route};
    match std::env::var("CANVAS_NATIVE_DEMO_PAGE").as_deref() {
        Ok("assignments") => Route::Course {
            id: "102".into(),
            tab: CourseTab::Assignments,
        },
        Ok("assignment") => Route::Assignment {
            course: "102".into(),
            id: "202".into(),
        },
        Ok("announcements") => Route::Course {
            id: "102".into(),
            tab: CourseTab::Announcements,
        },
        Ok("announcement") => Route::Announcement {
            course: "102".into(),
            id: "602".into(),
        },
        Ok("modules") => Route::Course {
            id: "101".into(),
            tab: CourseTab::Modules,
        },
        Ok("home") => Route::Course {
            id: "101".into(),
            tab: CourseTab::Home,
        },
        Ok("front-page") => Route::Course {
            id: "102".into(),
            tab: CourseTab::Home,
        },
        Ok("grades") => Route::Course {
            id: "102".into(),
            tab: CourseTab::Grades,
        },
        Ok("calendar") => Route::Calendar,
        Ok("document") => Route::Document {
            course: Some("101".into()),
            resource: "canvas-text://files/401/download".into(),
            title: "Assignment guidelines.pdf".into(),
            start: 0,
        },
        _ => Route::Dashboard,
    }
}

fn due(days: i64) -> String {
    format!(
        "{}T18:00:00Z",
        Local::now().date_naive() + chrono::Duration::days(days)
    )
}
fn courses() -> Vec<Value> {
    vec![
        json!({"id":"101","name":"Introduction to Environmental Science","course_code":"ENVS 101","workflow_state":"available","term":{"name":"Sample semester"},"teachers":[{"display_name":"Dr. Morgan"}],"enrollments":[{"computed_current_score":94.2}]}),
        json!({"id":"102","name":"Calculus II","course_code":"MATH 204","workflow_state":"available","term":{"name":"Sample semester"},"teachers":[{"display_name":"Dr. Chen"}],"enrollments":[{"computed_current_score":89.5}]}),
        json!({"id":"103","name":"World History","course_code":"HIST 110","workflow_state":"available","term":{"name":"Sample semester"},"teachers":[{"display_name":"Dr. Rivera"}]}),
        json!({"id":"99","name":"Academic Writing","course_code":"ENGL 101","workflow_state":"completed","concluded":true,"term":{"name":"Previous semester"}}),
    ]
}
fn assignments(course: &str) -> Vec<Value> {
    let (id, title, second) = match course {
        "102" => (
            "202",
            "Problem set 6 · Applications of integration",
            "Integration techniques quiz",
        ),
        "103" => ("204", "Primary source analysis", "Reading response"),
        _ => ("201", "Ecosystems field report", "Biodiversity quiz"),
    };
    vec![
        json!({"id":id,"course_id":course,"name":title,"due_at":due(2),"points_possible":100,"grading_type":"points","submission_types":["online_upload"],"submission":{"workflow_state":"unsubmitted","missing":false}}),
        json!({"id":format!("{id}0"),"course_id":course,"name":second,"due_at":due(-3),"points_possible":20,"submission_types":["online_quiz"],"submission":{"workflow_state":"graded","score":19,"grade":"19","missing":false}}),
    ]
}
fn announcements(course: &str) -> Vec<Value> {
    let (author, topic, newest, older) = match course {
        "102" => ("Dr. Chen", "Problem set 6 and Thursday lab", "602", "601"),
        "103" => ("Dr. Rivera", "Primary source workshop", "604", "603"),
        _ => ("Dr. Morgan", "Field trip reminder", "606", "605"),
    };
    vec![
        json!({"id":newest,"title":topic,"message":format!("<h2>{topic}</h2><p>{author} posted this update for the class.</p><p>Review the attached guidance before the next meeting.</p>"),"posted_at":due(0),"author":{"display_name":author},"html_url":format!("https://canvas.example.edu/courses/{course}/discussion_topics/{newest}"),"read_state":"unread"}),
        json!({"id":older,"title":"Welcome to the course","message":"<p>Course materials are available in Modules. Contact your instructor if you have questions.</p>","posted_at":due(-7),"author":{"display_name":author},"html_url":format!("https://canvas.example.edu/courses/{course}/discussion_topics/{older}"),"read_state":"read"}),
    ]
}
pub fn respond(operation: &Operation) -> Result<Value> {
    let Operation::Tool { name, args } = operation else {
        bail!("This action is not performed in the sample account.");
    };
    let course = args["course_id"].as_str().unwrap_or("101");
    let value = match name.as_str() {
        "document_preview" => {
            let page = args["page"].as_u64().unwrap_or(1);
            let mut bytes = Vec::new();
            {
                let mut encoder = png::Encoder::new(&mut bytes, 720, 960);
                encoder.set_color(png::ColorType::Rgb);
                encoder.set_depth(png::BitDepth::Eight);
                let mut writer = encoder.write_header()?;
                let pixels: Vec<_> = (0..960)
                    .flat_map(|y| {
                        (0..720).flat_map(move |x| {
                            if (40..680).contains(&x) && (40..130).contains(&y) {
                                if page == 1 {
                                    [38, 123, 150]
                                } else {
                                    [37, 131, 107]
                                }
                            } else if (55..650).contains(&x)
                                && (170..420).contains(&y)
                                && y % 40 < 7
                            {
                                [92, 104, 113]
                            } else if (80..640).contains(&x) && (500..790).contains(&y) {
                                if (x / 80 + y / 80) % 2 == 0 {
                                    [220, 235, 241]
                                } else {
                                    [180, 209, 221]
                                }
                            } else {
                                [255, 255, 255]
                            }
                        })
                    })
                    .collect();
                writer.write_image_data(&pixels)?;
            }
            return Ok(
                json!({"structuredContent":{"document":{"resource":args["resource"],"kind":"pdf","filename":"Sample document.pdf","page":page,"pages":2,"image_mime_type":"image/png","image_data":STANDARD.encode(bytes)}}}),
            );
        }
        "course_list" => json!({"courses":courses()}),
        "course_info" => {
            let mut course = courses()
                .into_iter()
                .find(|v| v["id"] == course)
                .unwrap_or_else(|| courses().remove(0));
            course["syllabus_body_html"] = json!(
                "<h2>Welcome to the course</h2><p>Use Modules to find readings and weekly activities. Assignment instructions and due dates are listed under Assignments.</p><h3>Assessment</h3><ul><li>Assignments: 40%</li><li>Weekly quizzes: 20%</li><li>Final project: 40%</li></ul><p>Contact your instructor during office hours if you need help getting started.</p>"
            );
            course["default_view"] = json!(if course["id"] == "102" {
                "wiki"
            } else {
                "modules"
            });
            json!({"course":course})
        }
        "course_front_page" => {
            json!({"page":{"title":"Welcome to your course","body_html":"<h2>Start here</h2><p>Your instructor’s home page appears here, rather than the syllabus.</p><ul><li>Read the weekly overview</li><li>Find assignments in Modules</li></ul>","front_page":true}})
        }
        "announcement_list" => json!({"announcements":announcements(course)}),
        "announcement_info" => {
            let aid = args["announcement_id"].as_str().unwrap_or("");
            let item = announcements(course)
                .into_iter()
                .find(|v| v["id"] == aid)
                .unwrap_or_else(|| announcements(course).remove(0));
            json!({"announcement":item})
        }
        "assignment_list" => json!({"assignments":assignments(course)}),
        "assignment_info" => {
            let aid = args["assignment_id"].as_str().unwrap_or("");
            let mut item = assignments(course)
                .into_iter()
                .find(|v| v["id"] == aid)
                .unwrap_or_else(|| assignments(course).remove(0));
            item["description_html"] = json!(match course {
                "102" =>
                    "<h2>Problem set 6</h2><p>Complete the assigned applications of integration. Show your setup, calculations, and units.</p><h3>Submission</h3><p>Upload your work as a single PDF. Consult the attached guidelines before submitting.</p>",
                "103" =>
                    "<h2>Primary source analysis</h2><p>Choose a primary source from this week’s readings and discuss its author, context, audience, and historical significance.</p><p>Support your analysis with specific evidence.</p>",
                _ =>
                    "<h2>Ecosystems field report</h2><p>Describe the relationships between organisms at your field site. Include your observations, a data table, and a discussion of biodiversity.</p><h3>What to include</h3><ul><li>A brief description of your field site</li><li>A table of observations</li><li>At least three ecological relationships</li><li>References for your sources</li></ul><p>Submit a PDF before the due date.</p>",
            });
            item["attachments"] = json!([{"id":"401","filename":"Assignment guidelines.pdf","size":48230,"content_type":"application/pdf","resource":"canvas-text://files/401/download","download_resource":"canvas://files/401/download"}]);
            json!({"assignment":item})
        }
        "dashboard_items" => {
            let mut items:Vec<Value>=courses().iter().filter(|c|model::course_is_current(c)).map(|c|{let assignment=assignments(c["id"].as_str().unwrap()).remove(0);json!({"item_id":assignment["id"],"course_id":c["id"],"course":c["course_code"],"item_type":"assignment","title":assignment["name"],"due_at":assignment["due_at"],"points_possible":100,"submitted":false})}).collect();
            items.push(json!({"item_id":"301","item_type":"planner_note","title":"Review notes before the lab","scheduled_at":due(1)}));
            if let Some(ids) = args["course_ids"].as_array() {
                items.retain(|v| ids.contains(&v["course_id"]));
            }
            if args["filter"] == "complete_items" {
                items.clear();
            }
            if let Some(start) = args["start_date"].as_str().and_then(model::date) {
                items.retain(|v| {
                    v["due_at"]
                        .as_str()
                        .or(v["scheduled_at"].as_str())
                        .and_then(model::date)
                        .is_none_or(|d| d >= start)
                });
            }
            if let Some(end) = args["end_date"].as_str().and_then(model::date) {
                items.retain(|v| {
                    v["due_at"]
                        .as_str()
                        .or(v["scheduled_at"].as_str())
                        .and_then(model::date)
                        .is_none_or(|d| d <= end)
                });
            }
            json!({"items":items})
        }
        "module_list" => {
            let assignment = assignments(course).remove(0);
            json!({"modules":[{"id":"501","name":"Getting Started","state":"completed","items":[{"title":"Welcome & course overview","item_type":"Page","page_url":"welcome","completion_requirement":{"completed":true}}]},{"id":"502","name":"This Week · Readings and Assignments","state":"started","items":[{"title":"Weekly reading","item_type":"Page","page_url":"weekly-reading"},{"title":assignment["name"],"item_type":"Assignment","content_id":assignment["id"],"content_details":{"due_at":assignment["due_at"],"points_possible":100},"completion_requirement":{"requirement_type":"must_submit","completed":false}},{"title":"Assignment guidelines.pdf","item_type":"File","content_id":"401"}]}]})
        }
        "page_info" => {
            json!({"page":{"title":if args["page_url"]=="welcome"{"Welcome & course overview"}else{"Weekly reading"},"body_html":"<h2>This week’s reading</h2><p>Read the assigned chapter before class. Take notes on the main ideas and bring two questions to discuss with your classmates.</p><h3>Learning objectives</h3><ul><li>Identify the key concepts from the chapter</li><li>Connect evidence to the ideas discussed in class</li><li>Apply these concepts to the weekly assignment</li></ul>","published":true}})
        }
        "grade_summary" => {
            json!({"grades":[{"course_id":"101","course_name":"Introduction to Environmental Science","course_code":"ENVS 101","current_score":94.2,"current_grade":"A","final_score":86.1},{"course_id":"102","course_name":"Calculus II","course_code":"MATH 204","current_score":89.5,"current_grade":"B+","final_score":78.4},{"course_id":"103","course_name":"World History","current_score":null,"final_score":null}]})
        }
        "course_grades" | "assignment_grade" => {
            let assignments:Vec<_>=assignments(course).into_iter().map(|a|json!({"assignment_id":a["id"],"name":a["name"],"score":a["submission"]["score"],"points_possible":a["points_possible"],"status":a["submission"]["workflow_state"],"due_at":a["due_at"]})).collect();
            if name == "assignment_grade" {
                let grade = assignments
                    .iter()
                    .find(|v| v["assignment_id"] == args["assignment_id"])
                    .cloned()
                    .unwrap_or(Value::Null);
                json!({"assignment_grade":grade,"total_grade":{"current_score":94.2}})
            } else {
                json!({"course_grades":{"course_id":course,"total_grade":{"current_score":if course=="102"{89.5}else{94.2},"current_grade":"A","final_score":86.1},"assignments":assignments}})
            }
        }
        "planner_note_info" => {
            json!({"note":{"id":"301","title":"Review notes before the lab","details":"Bring the field notebook and review sampling methods.","todo_date":due(1),"course_id":"101"}})
        }
        "file_info" => {
            json!({"file":{"id":"401","display_name":"Assignment guidelines.pdf","filename":"Assignment guidelines.pdf","content_type":"application/pdf","size":48230,"resource":"canvas-text://files/401/download","download_resource":"canvas://files/401/download"}})
        }
        "attachment_text" => {
            json!({"resource":args["resource"],"text":"Assignment guidelines\n\n1. Introduce the topic and describe your approach.\n2. Present your observations or calculations.\n3. Discuss your findings using course concepts.\n4. Cite your sources.\n\nThe figure below is sample document content.","start":0,"end":230,"total_chars":230,"next_start":null,"images":[{"page":1,"image":1,"width":320,"height":160,"supported":true}]})
        }
        "attachment_image" => {
            let mut bytes = Vec::new();
            {
                let mut encoder = png::Encoder::new(&mut bytes, 320, 160);
                encoder.set_color(png::ColorType::Rgb);
                encoder.set_depth(png::BitDepth::Eight);
                let mut writer = encoder.write_header()?;
                let pixels: Vec<_> = (0..160)
                    .flat_map(|y| {
                        (0..320).flat_map(move |x| {
                            if (x / 40 + y / 40) % 2 == 0 {
                                [46, 124, 152]
                            } else {
                                [225, 238, 243]
                            }
                        })
                    })
                    .collect();
                writer.write_image_data(&pixels)?;
            }
            return Ok(
                json!({"content":[{"type":"image","mimeType":"image/png","data":STANDARD.encode(bytes)}]}),
            );
        }
        _ => bail!("The sample account does not sign in, download files, or change Canvas data."),
    };
    Ok(json!({"structuredContent":value}))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn course_and_assignment_selections_are_not_interchangeable() {
        for id in ["101", "102", "103"] {
            let data = respond(&Operation::Tool {
                name: "assignment_list".into(),
                args: json!({"course_id":id}),
            })
            .unwrap();
            assert_eq!(data["structuredContent"]["assignments"][0]["course_id"], id);
        }
    }
    #[test]
    fn announcements_are_scoped_to_the_selected_course() {
        for id in ["101", "102", "103"] {
            let data = respond(&Operation::Tool {
                name: "announcement_list".into(),
                args: json!({"course_id":id}),
            })
            .unwrap();
            let announcements = data["structuredContent"]["announcements"]
                .as_array()
                .unwrap();
            assert_eq!(announcements.len(), 2);
            assert!(
                announcements[0]["html_url"]
                    .as_str()
                    .unwrap()
                    .contains(&format!("/courses/{id}/"))
            );
        }
    }
    #[test]
    fn missing_grades_remain_unavailable() {
        let data = respond(&Operation::Tool {
            name: "grade_summary".into(),
            args: json!({}),
        })
        .unwrap();
        assert!(data["structuredContent"]["grades"][2]["current_score"].is_null());
    }
}
