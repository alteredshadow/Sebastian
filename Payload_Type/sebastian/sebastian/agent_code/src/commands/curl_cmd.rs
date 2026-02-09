use crate::structs::Task;
use serde::Deserialize;
use std::collections::HashMap;

#[derive(Deserialize)]
struct CurlArgs {
    url: String,
    #[serde(default = "default_method")]
    method: String,
    #[serde(default)]
    headers: HashMap<String, String>,
    #[serde(default)]
    body: String,
}

fn default_method() -> String { "GET".to_string() }

pub async fn execute(task: Task) {
    let mut response = task.new_response();
    let args: CurlArgs = match serde_json::from_str(&task.data.params) {
        Ok(a) => a,
        Err(e) => {
            response.set_error(&format!("Failed to parse: {}", e));
            let _ = task.job.send_responses.send(response).await;
            let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
            return;
        }
    };

    let client = reqwest::Client::builder()
        .danger_accept_invalid_certs(true)
        .build()
        .unwrap_or_else(|_| reqwest::Client::new());

    let method = match args.method.to_uppercase().as_str() {
        "GET" => reqwest::Method::GET,
        "POST" => reqwest::Method::POST,
        "PUT" => reqwest::Method::PUT,
        "DELETE" => reqwest::Method::DELETE,
        "PATCH" => reqwest::Method::PATCH,
        "HEAD" => reqwest::Method::HEAD,
        "OPTIONS" => reqwest::Method::OPTIONS,
        _ => reqwest::Method::GET,
    };

    let mut req = client.request(method, &args.url);
    for (k, v) in &args.headers {
        req = req.header(k.as_str(), v.as_str());
    }
    if !args.body.is_empty() {
        req = req.body(args.body);
    }

    match req.send().await {
        Ok(resp) => {
            let status = resp.status();
            let headers = resp.headers().clone();
            let body = resp.text().await.unwrap_or_default();
            let mut output = format!("Status: {}\n\nHeaders:\n", status);
            for (k, v) in &headers {
                output.push_str(&format!("  {}: {}\n", k, v.to_str().unwrap_or("")));
            }
            output.push_str(&format!("\nBody:\n{}", body));
            response.user_output = output;
            response.completed = true;
        }
        Err(e) => response.set_error(&format!("Request failed: {}", e)),
    }

    let _ = task.job.send_responses.send(response).await;
    let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
}
