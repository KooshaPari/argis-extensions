//! Example: run a chat completion against the vendored Bifrost (mocked echo provider).
//!
//! Run with: `cargo run --release --example chat` from the bifrost-ffi dir.

fn main() {
    let prompt = "Tell me a joke about Rust.";
    let model = "gpt-4o-mini";
    println!("prompt: {prompt}");
    println!("model:  {model}");
    let env = argis_bifrost_ffi::chat_completion(model, prompt)
        .expect("chat_completion failed");
    println!("\nresponse content:");
    println!("  {}", env.content().unwrap_or("<no content>"));
    println!("\nraw JSON envelope:");
    println!("{}", serde_json::to_string_pretty(&serde_json::json!({
        "request": env.request,
        "response": env.response,
    })).unwrap());
}
