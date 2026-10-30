//! Exercise the real Linux tools through Rabbit's process and post-processing code.
#![cfg(target_os = "linux")]

use std::ffi::OsString;
use std::path::Path;

use rabbit_lib::engine::post;
use rabbit_lib::sys::process::{command, spawn_contained};
use rabbit_lib::sys::tools::Tools;
use tokio_util::sync::CancellationToken;

async fn fixture(ffmpeg: &Path, args: &[&str], output: &Path) {
    let mut args: Vec<OsString> = args.iter().map(OsString::from).collect();
    args.push(output.into());
    let proc = spawn_contained(command(ffmpeg, args)).unwrap();
    let out = proc.child.wait_with_output().await.unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
}

#[tokio::test]
async fn native_tools_merge_convert_and_verify_paths_with_spaces() {
    let tools = Tools::discover();
    let ffmpeg = tools.ffmpeg.as_deref().expect("prepare tools with scripts/fetch-sidecars-linux.py");
    let tmp = tempfile::tempdir().unwrap();
    let video = tmp.path().join("test video.mp4");
    let audio = tmp.path().join("test audio.m4a");
    let merged = tmp.path().join("merged video.mp4");
    let mp3 = tmp.path().join("converted audio.mp3");
    fixture(ffmpeg, &["-hide_banner", "-nostdin", "-loglevel", "error", "-f", "lavfi", "-i", "color=c=blue:s=320x240:r=25", "-t", "1", "-c:v", "mpeg4", "-y"], &video).await;
    fixture(ffmpeg, &["-hide_banner", "-nostdin", "-loglevel", "error", "-f", "lavfi", "-i", "sine=frequency=440", "-t", "1", "-c:a", "aac", "-y"], &audio).await;

    let cancel = CancellationToken::new();
    post::merge(&tools, &video, &audio, &merged, &cancel).await.unwrap();
    post::verify(&tools, &merged, Some(1.0), true, true, Some(240), &cancel).await.unwrap();
    let info = post::probe(&tools, &merged, &cancel).await.unwrap();
    assert_eq!((info.video, info.audio), (1, 1));
    assert_eq!((info.width, info.height), (Some(320), Some(240)));
    assert!(post::verify(&tools, &merged, Some(1.0), true, true, Some(720), &cancel).await.is_err());

    post::to_mp3(&tools, &merged, &mp3, &cancel).await.unwrap();
    post::verify(&tools, &mp3, Some(1.0), false, true, None, &cancel).await.unwrap();
    let info = post::probe(&tools, &mp3, &cancel).await.unwrap();
    assert_eq!((info.video, info.audio), (0, 1));
    assert_eq!(info.acodec.as_deref(), Some("mp3"));
}
