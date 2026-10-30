use std::path::{Path, PathBuf};

use serde::Serialize;

use super::updater;

/// Sidecars take priority. Linux also supports distribution-managed tools on PATH.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Tools {
    /// The yt-dlp in use: a verified update when one is active, otherwise the bundled copy.
    pub ytdlp: Option<PathBuf>,
    pub bundled_ytdlp: Option<PathBuf>,
    /// Version of the active updated copy (from its verified manifest); `None` when bundled.
    pub ytdlp_update: Option<updater::Installed>,
    pub ffmpeg: Option<PathBuf>,
    pub ffprobe: Option<PathBuf>,
    /// `RUNTIME:PATH` value for yt-dlp's `--js-runtimes` (needed for full YouTube support).
    pub js_runtime: Option<String>,
}

impl Tools {
    pub fn discover() -> Self {
        Self::discover_with(None)
    }

    /// `update_dir` is where verified yt-dlp updates are kept (see `sys::updater`).
    pub fn discover_with(update_dir: Option<&Path>) -> Self {
        let mut tools = Self::discover_in(None);
        if let Some(dir) = update_dir { tools.apply_update_dir(dir); }
        tools
    }

    /// Linux packages keep private tools in the resource directory, avoiding /usr/bin collisions.
    pub fn discover_in(resource_dir: Option<&Path>) -> Self {
        let exe_dir = std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf));
        let mut dirs = Vec::new();
        if let Some(resources) = resource_dir { dirs.push(resources.join("tools")); }
        if let Some(dir) = exe_dir { dirs.push(dir); }
        #[cfg(all(debug_assertions, target_os = "linux"))]
        dirs.push(Path::new(env!("CARGO_MANIFEST_DIR")).join("binaries/linux"));
        let local = |name: &str| dirs.iter().map(|d| d.join(name)).find(|p| is_executable(p));
        let find = |name: &str| local(name).or_else(|| if cfg!(any(debug_assertions, target_os = "linux")) { on_path(name) } else { None });

        let js_runtime = local(&executable_name("deno"))
            .map(|p| format!("deno:{}", p.display()))
            .or_else(|| local(&executable_name("node")).map(|p| format!("node:{}", p.display())))
            .or_else(|| if cfg!(any(debug_assertions, target_os = "linux")) { on_path(&executable_name("deno")).map(|p| format!("deno:{}", p.display())).or_else(|| on_path(&executable_name("node")).map(|p| format!("node:{}", p.display()))) } else { None });

        let bundled = find(&executable_name("yt-dlp"));
        Tools { ytdlp: bundled.clone(), bundled_ytdlp: bundled, ytdlp_update: None, ffmpeg: find(&executable_name("ffmpeg")), ffprobe: find(&executable_name("ffprobe")), js_runtime }
    }

    /// Re-reads the update folder: uses the verified update if one is active, else the bundled copy.
    pub fn apply_update_dir(&mut self, dir: &Path) {
        match updater::active(dir) {
            Some((installed, path)) => {
                self.ytdlp = Some(path);
                self.ytdlp_update = Some(installed);
            }
            None => {
                self.ytdlp = self.bundled_ytdlp.clone();
                self.ytdlp_update = None;
            }
        }
    }
}

fn on_path(name: &str) -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?).map(|d| d.join(name)).find(|p| is_executable(p))
}

/// Executable names follow the host platform, including yt-dlp's downloaded updates.
pub fn executable_name(stem: &str) -> String {
    format!("{stem}{}", std::env::consts::EXE_SUFFIX)
}

pub fn is_executable(path: &Path) -> bool {
    if !path.is_file() { return false; }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        path.metadata().is_ok_and(|m| m.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    { true }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn bundled_linux_tools_take_priority_over_system_tools() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("tools");
        std::fs::create_dir(&dir).unwrap();
        for name in ["yt-dlp", "ffmpeg", "ffprobe", "deno"] {
            let path = dir.join(name);
            std::fs::write(&path, "#!/bin/sh\nexit 0\n").unwrap();
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let tools = Tools::discover_in(Some(root.path()));
        assert_eq!(tools.ytdlp, Some(dir.join("yt-dlp")));
        assert_eq!(tools.ffmpeg, Some(dir.join("ffmpeg")));
        assert_eq!(tools.ffprobe, Some(dir.join("ffprobe")));
        assert_eq!(tools.js_runtime, Some(format!("deno:{}", dir.join("deno").display())));
    }

    #[test]
    fn linux_tools_need_execute_permission() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("yt-dlp");
        std::fs::write(&path, "binary").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(!is_executable(&path));
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(is_executable(&path));
        assert_eq!(executable_name("ffmpeg"), "ffmpeg");
        assert!(on_path("sh").is_some());
    }
}
