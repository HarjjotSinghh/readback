//! Readback's reference desktop app.
//!
//! This is a demo for the SDK, not a dictation product. It has no microphone:
//! it shows what the decision layer does and what the overlay should look like,
//! which is the part the SDK actually owns. A real host app supplies the audio,
//! the hotkey and the paste.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;

use app::{Request, Scenario, VerdictView};
use readback_core::SpeechRegion;
use readback_bench::vad;
use serde::Serialize;
use std::path::PathBuf;

#[tauri::command]
fn scenarios() -> Vec<Scenario> {
    app::scenarios()
}

#[tauri::command]
fn check(request: Request) -> VerdictView {
    app::check(&request)
}

/// What loading a clip gives the window: the speech regions, and enough detail
/// to draw them.
#[derive(Debug, Serialize)]
struct ClipView {
    duration_ms: u32,
    sample_rate: u32,
    speech: Vec<SpeechRegion>,
}

#[tauri::command]
fn load_clip(path: String) -> Result<ClipView, String> {
    let clip = vad::read_wav(&PathBuf::from(path))?;
    let speech = vad::detect(&clip, &vad::VadConfig::default());
    Ok(ClipView { duration_ms: clip.duration_ms(), sample_rate: clip.sample_rate, speech })
}

#[tauri::command]
fn version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![scenarios, check, load_clip, version])
        .run(tauri::generate_context!())
        .expect("failed to start the Readback demo");
}
