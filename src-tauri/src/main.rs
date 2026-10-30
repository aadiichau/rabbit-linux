// Hide the console window in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    #[cfg(target_os = "linux")]
    {
        let backend = std::env::var("GDK_BACKEND").ok();
        let x11 = backend.as_deref() == Some("x11")
            || (backend.is_none() && std::env::var("XDG_SESSION_TYPE").as_deref() == Ok("x11"));
        if x11
            && std::path::Path::new("/sys/module/nvidia").is_dir()
            && std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none()
        {
            // NVIDIA's GBM path can leave WebKitGTK's X11 surface blank. Use its fallback
            // renderer on this combination; native Wayland and explicit overrides stay intact.
            // SAFETY: the main thread has not started Tauri, GTK, or any worker threads yet.
            unsafe { std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1") };
        }
    }
    rabbit_lib::run();
}
