//! Pixtral AI Image Generation WASM Tool for IronClaw.
//!
//! Generates images using the Mistral AI Agents API with the built-in
//! image_generation tool (powered by Black Forest Labs FLUX 1.1 [pro] Ultra).
//!
//! Flow:
//!   1. Create an agent with `image_generation` tool enabled
//!   2. Start a conversation with the prompt
//!   3. Extract the file_id from the response
//!   4. Download the image and return it as base64

wit_bindgen::generate!({
    world: "sandboxed-tool",
    path: "wit/tool.wit",
});

use serde::{Deserialize, Serialize};

const API_BASE: &str = "https://api.mistral.ai/v1";
const AGENT_MODEL: &str = "mistral-medium-latest";
const TIMEOUT_MS: u32 = 120_000;

struct PixtralTool;

impl exports::near::agent::tool::Guest for PixtralTool {
    fn execute(req: exports::near::agent::tool::Request) -> exports::near::agent::tool::Response {
        match execute_inner(&req.params) {
            Ok(result) => exports::near::agent::tool::Response {
                output: Some(result),
                error: None,
            },
            Err(e) => exports::near::agent::tool::Response {
                output: None,
                error: Some(e),
            },
        }
    }

    fn schema() -> String {
        SCHEMA.to_string()
    }

    fn description() -> String {
        "Generate images from text descriptions using Mistral AI's image generation. \
         Provide a detailed text prompt and receive a base64-encoded PNG image. \
         The underlying model is FLUX 1.1 [pro] Ultra by Black Forest Labs."
            .to_string()
    }
}

#[derive(Debug, Deserialize)]
struct GenerateParams {
    prompt: String,
}

// ── Agent API types ──────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
struct CreateAgentRequest {
    model: &'static str,
    name: &'static str,
    instructions: &'static str,
    tools: Vec<AgentTool>,
}

#[derive(Debug, Serialize)]
struct AgentTool {
    r#type: &'static str,
}

#[derive(Debug, Deserialize)]
struct AgentResponse {
    id: String,
}

// ── Conversation API types ───────────────────────────────────────────────────

#[derive(Debug, Serialize)]
struct ConversationRequest {
    inputs: String,
    stream: bool,
    agent_id: String,
}

#[derive(Debug, Deserialize)]
struct ConversationResponse {
    #[serde(default)]
    outputs: Vec<OutputEntry>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum OutputEntry {
    #[serde(rename = "message.output")]
    MessageOutput {
        #[serde(default)]
        content: Vec<ContentPart>,
    },
    #[serde(other)]
    Other,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum ContentPart {
    #[serde(rename = "tool_file")]
    ToolFile {
        file_id: String,
        #[serde(default)]
        _file_type: Option<String>,
    },
    #[serde(other)]
    Other,
}

fn execute_inner(params: &str) -> Result<String, String> {
    let params: GenerateParams =
        serde_json::from_str(params).map_err(|e| format!("Invalid parameters: {e}"))?;

    if params.prompt.is_empty() {
        return Err("'prompt' must not be empty".into());
    }
    if params.prompt.len() > 4000 {
        return Err("'prompt' exceeds maximum length of 4000 characters".into());
    }

    near::agent::host::log(
        near::agent::host::LogLevel::Info,
        &format!(
            "Generating image for prompt: {}",
            truncate(&params.prompt, 100)
        ),
    );

    // Step 1: Create an agent with image_generation tool
    let agent_id = create_agent()?;

    near::agent::host::log(
        near::agent::host::LogLevel::Debug,
        &format!("Created agent: {agent_id}"),
    );

    // Step 2: Start a conversation to generate the image
    let file_id = generate_image(&agent_id, &params.prompt)?;

    near::agent::host::log(
        near::agent::host::LogLevel::Debug,
        &format!("Got file_id: {file_id}"),
    );

    // Step 3: Download the generated image
    let image_bytes = download_file(&file_id)?;

    near::agent::host::log(
        near::agent::host::LogLevel::Info,
        &format!("Downloaded image: {} bytes", image_bytes.len()),
    );

    // Step 4: Cleanup - delete the agent
    let _ = delete_agent(&agent_id);

    // Return as base64
    let base64 = base64_encode(&image_bytes);
    let output = serde_json::json!({
        "prompt": params.prompt,
        "image_base64": base64,
        "mime_type": "image/png",
        "size_bytes": image_bytes.len(),
    });

    serde_json::to_string(&output).map_err(|e| format!("Failed to serialize output: {e}"))
}

fn create_agent() -> Result<String, String> {
    let body = CreateAgentRequest {
        model: AGENT_MODEL,
        name: "ironclaw-image-gen",
        instructions: "Generate images exactly as described. Do not add extra details.",
        tools: vec![AgentTool {
            r#type: "image_generation",
        }],
    };

    let body_json = serde_json::to_string(&body)
        .map_err(|e| format!("Failed to serialize agent request: {e}"))?;

    let headers = serde_json::json!({
        "Content-Type": "application/json",
        "Accept": "application/json"
    });

    let resp = near::agent::host::http_request(
        "POST",
        &format!("{API_BASE}/agents"),
        &headers.to_string(),
        Some(body_json.as_bytes()),
        Some(TIMEOUT_MS),
    )
    .map_err(|e| format!("Failed to create agent: {e}"))?;

    if resp.status < 200 || resp.status >= 300 {
        let body = String::from_utf8_lossy(&resp.body);
        return Err(format!(
            "Failed to create agent (HTTP {}): {body}",
            resp.status
        ));
    }

    let agent: AgentResponse = serde_json::from_slice(&resp.body)
        .map_err(|e| format!("Failed to parse agent response: {e}"))?;

    Ok(agent.id)
}

fn generate_image(agent_id: &str, prompt: &str) -> Result<String, String> {
    let body = ConversationRequest {
        inputs: prompt.to_string(),
        stream: false,
        agent_id: agent_id.to_string(),
    };

    let body_json = serde_json::to_string(&body)
        .map_err(|e| format!("Failed to serialize conversation request: {e}"))?;

    let headers = serde_json::json!({
        "Content-Type": "application/json",
        "Accept": "application/json"
    });

    let resp = near::agent::host::http_request(
        "POST",
        &format!("{API_BASE}/conversations"),
        &headers.to_string(),
        Some(body_json.as_bytes()),
        Some(TIMEOUT_MS),
    )
    .map_err(|e| format!("Failed to generate image: {e}"))?;

    if resp.status < 200 || resp.status >= 300 {
        let body = String::from_utf8_lossy(&resp.body);
        return Err(format!(
            "Image generation failed (HTTP {}): {body}",
            resp.status
        ));
    }

    let conversation: ConversationResponse = serde_json::from_slice(&resp.body)
        .map_err(|e| format!("Failed to parse conversation response: {e}"))?;

    for output in &conversation.outputs {
        if let OutputEntry::MessageOutput { content } = output {
            for part in content {
                if let ContentPart::ToolFile { file_id, .. } = part {
                    return Ok(file_id.clone());
                }
            }
        }
    }

    Err("No image file found in the generation response".into())
}

fn download_file(file_id: &str) -> Result<Vec<u8>, String> {
    let headers = serde_json::json!({
        "Accept": "application/octet-stream"
    });

    let resp = near::agent::host::http_request(
        "GET",
        &format!("{API_BASE}/files/{file_id}/content"),
        &headers.to_string(),
        None,
        Some(TIMEOUT_MS),
    )
    .map_err(|e| format!("Failed to download image: {e}"))?;

    if resp.status < 200 || resp.status >= 300 {
        let body = String::from_utf8_lossy(&resp.body);
        return Err(format!(
            "Image download failed (HTTP {}): {body}",
            resp.status
        ));
    }

    if resp.body.is_empty() {
        return Err("Downloaded image is empty".into());
    }

    Ok(resp.body)
}

fn delete_agent(agent_id: &str) -> Result<(), String> {
    let headers = serde_json::json!({
        "Accept": "application/json"
    });

    near::agent::host::http_request(
        "DELETE",
        &format!("{API_BASE}/agents/{agent_id}"),
        &headers.to_string(),
        None,
        Some(30_000),
    )
    .map_err(|e| format!("Failed to delete agent: {e}"))?;

    Ok(())
}

fn truncate(s: &str, max: usize) -> &str {
    if s.len() <= max {
        s
    } else {
        &s[..max]
    }
}

// Simple base64 encoder (no external dependency needed)
fn base64_encode(data: &[u8]) -> String {
    const CHARS: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);

    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let triple = (b0 << 16) | (b1 << 8) | b2;

        out.push(CHARS[((triple >> 18) & 0x3F) as usize] as char);
        out.push(CHARS[((triple >> 12) & 0x3F) as usize] as char);

        if chunk.len() > 1 {
            out.push(CHARS[((triple >> 6) & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }

        if chunk.len() > 2 {
            out.push(CHARS[(triple & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
    }

    out
}

const SCHEMA: &str = r#"{
    "type": "object",
    "properties": {
        "prompt": {
            "type": "string",
            "description": "A detailed text description of the image to generate. Be specific about style, subject, composition, lighting, etc."
        }
    },
    "required": ["prompt"],
    "additionalProperties": false
}"#;

export!(PixtralTool);
