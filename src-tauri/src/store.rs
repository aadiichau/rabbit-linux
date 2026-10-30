use std::path::{Path, PathBuf};
use std::sync::Mutex;

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};

impl From<rusqlite::Error> for AppError {
    fn from(e: rusqlite::Error) -> Self {
        AppError::Io(format!("database: {e}"))
    }
}

const MIGRATIONS: &[&str] = &[r#"
CREATE TABLE library (
  id INTEGER PRIMARY KEY,
  source_url TEXT NOT NULL,
  platform TEXT NOT NULL,
  title TEXT NOT NULL,
  uploader TEXT,
  kind TEXT NOT NULL,
  option_label TEXT NOT NULL,
  container TEXT NOT NULL,
  file_path TEXT NOT NULL,
  file_size INTEGER NOT NULL,
  duration REAL,
  completed_at INTEGER NOT NULL
);
CREATE INDEX library_completed ON library(completed_at DESC);
CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
"#, r#"
CREATE TABLE tasks (id TEXT PRIMARY KEY, payload TEXT NOT NULL, created_at INTEGER NOT NULL);
"#];

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub download_dir: PathBuf,
    pub max_concurrent: usize,
    pub max_segments: usize,
    /// Desktop notification when a download finishes or fails while Rabbit isn't focused.
    #[serde(default = "yes")]
    pub notifications: bool,
    /// Release channel used for yt-dlp update checks.
    #[serde(default)]
    pub ytdlp_channel: crate::sys::updater::Channel,
    /// Opt-in signed-in sessions (off by default). Holds the source and switches, never cookies.
    #[serde(default)]
    pub sessions: crate::sys::cookies::Sessions,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct LibraryQuery {
    pub text: String,
    pub platform: Option<String>,
    pub kind: Option<String>,
    /// "newest" (default) | "oldest" | "largest" | "title"
    pub sort: Option<String>,
    pub offset: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Facet {
    pub value: String,
    pub count: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryFacets {
    pub total: u64,
    pub platforms: Vec<Facet>,
    pub kinds: Vec<Facet>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryItem {
    pub id: i64,
    pub source_url: String,
    pub platform: String,
    pub title: String,
    pub uploader: Option<String>,
    pub kind: String,
    pub option_label: String,
    pub container: String,
    pub file_path: String,
    pub file_size: u64,
    pub duration: Option<f64>,
    pub completed_at: i64,
    pub file_exists: bool,
}

pub struct NewLibraryItem<'a> {
    pub source_url: &'a str,
    pub platform: &'a str,
    pub title: &'a str,
    pub uploader: Option<&'a str>,
    pub kind: &'a str,
    pub option_label: &'a str,
    pub container: &'a str,
    pub file_path: &'a Path,
    pub file_size: u64,
    pub duration: Option<f64>,
}

pub struct Store {
    conn: Mutex<Connection>,
}

impl Store {
    pub fn open(path: &Path) -> AppResult<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let conn = Connection::open(path)?;
        Self::init(conn)
    }

    #[cfg(test)]
    pub fn memory() -> AppResult<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> AppResult<Self> {
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
        for (i, m) in MIGRATIONS.iter().enumerate().skip(version as usize) {
            conn.execute_batch(m)?;
            conn.pragma_update(None, "user_version", (i + 1) as i64)?;
        }
        Ok(Store { conn: Mutex::new(conn) })
    }

    pub fn load_settings(&self) -> AppResult<Option<Settings>> {
        let c = self.conn.lock().unwrap();
        let raw: Option<String> = c.query_row("SELECT value FROM settings WHERE key = 'app'", [], |r| r.get(0)).optional()?;
        Ok(raw.and_then(|r| serde_json::from_str(&r).ok()))
    }

    pub fn save_settings(&self, s: &Settings) -> AppResult<()> {
        let json = serde_json::to_string(s).map_err(|e| AppError::Io(e.to_string()))?;
        self.conn.lock().unwrap().execute("INSERT INTO settings(key, value) VALUES('app', ?1) ON CONFLICT(key) DO UPDATE SET value = excluded.value", [json])?;
        Ok(())
    }

    pub fn add(&self, it: &NewLibraryItem) -> AppResult<i64> {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0);
        let c = self.conn.lock().unwrap();
        c.execute(
            "INSERT INTO library(source_url, platform, title, uploader, kind, option_label, container, file_path, file_size, duration, completed_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![it.source_url, it.platform, it.title, it.uploader, it.kind, it.option_label, it.container, it.file_path.to_string_lossy(), it.file_size as i64, it.duration, now],
        )?;
        Ok(c.last_insert_rowid())
    }

    pub fn save_task(&self, id: &str, payload: &str, created_at: i64) -> AppResult<()> {
        self.conn.lock().unwrap().execute(
            "INSERT INTO tasks(id, payload, created_at) VALUES (?1, ?2, ?3) ON CONFLICT(id) DO UPDATE SET payload = excluded.payload",
            params![id, payload, created_at],
        )?;
        Ok(())
    }

    pub fn delete_task(&self, id: &str) -> AppResult<()> {
        self.conn.lock().unwrap().execute("DELETE FROM tasks WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn load_tasks(&self) -> AppResult<Vec<String>> {
        let c = self.conn.lock().unwrap();
        let mut st = c.prepare("SELECT payload FROM tasks ORDER BY created_at")?;
        let rows = st.query_map([], |r| r.get(0))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Text matches title or uploader (case-insensitive substring); platform/kind are exact filters.
    pub fn list(&self, q: &LibraryQuery, limit: usize) -> AppResult<Vec<LibraryItem>> {
        let c = self.conn.lock().unwrap();
        let like = format!("%{}%", q.text.trim().replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_"));
        // ORDER BY comes from a fixed list, never from user text.
        let order = match q.sort.as_deref() {
            Some("oldest") => "completed_at ASC",
            Some("largest") => "file_size DESC, completed_at DESC",
            Some("title") => "title COLLATE NOCASE ASC",
            _ => "completed_at DESC",
        };
        let sql = format!(
            "SELECT id, source_url, platform, title, uploader, kind, option_label, container, file_path, file_size, duration, completed_at
             FROM library
             WHERE (?1 = '%%' OR title LIKE ?1 ESCAPE '\\' OR uploader LIKE ?1 ESCAPE '\\')
               AND (?2 IS NULL OR platform = ?2)
               AND (?3 IS NULL OR kind = ?3)
             ORDER BY {order} LIMIT ?4 OFFSET ?5"
        );
        let mut st = c.prepare(&sql)?;
        let rows = st.query_map(params![like, q.platform, q.kind, limit as i64, q.offset as i64], |r| {
            let file_path: String = r.get(8)?;
            Ok(LibraryItem {
                id: r.get(0)?,
                source_url: r.get(1)?,
                platform: r.get(2)?,
                title: r.get(3)?,
                uploader: r.get(4)?,
                kind: r.get(5)?,
                option_label: r.get(6)?,
                container: r.get(7)?,
                file_exists: Path::new(&file_path).exists(),
                file_path,
                file_size: r.get::<_, i64>(9)? as u64,
                duration: r.get(10)?,
                completed_at: r.get(11)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn facets(&self) -> AppResult<LibraryFacets> {
        let c = self.conn.lock().unwrap();
        let group = |col: &str| -> AppResult<Vec<Facet>> {
            let mut st = c.prepare(&format!("SELECT {col}, COUNT(*) FROM library GROUP BY {col} ORDER BY COUNT(*) DESC"))?;
            let rows = st.query_map([], |r| Ok(Facet { value: r.get(0)?, count: r.get::<_, i64>(1)? as u64 }))?;
            Ok(rows.collect::<Result<_, _>>()?)
        };
        let total: i64 = c.query_row("SELECT COUNT(*) FROM library", [], |r| r.get(0))?;
        Ok(LibraryFacets { total: total as u64, platforms: group("platform")?, kinds: group("kind")? })
    }

    pub fn get_path(&self, id: i64) -> AppResult<PathBuf> {
        let c = self.conn.lock().unwrap();
        let p: Option<String> = c.query_row("SELECT file_path FROM library WHERE id = ?1", [id], |r| r.get(0)).optional()?;
        p.map(PathBuf::from).ok_or_else(|| AppError::NotFound("That library item no longer exists.".into()))
    }

    pub fn remove(&self, id: i64) -> AppResult<()> {
        self.conn.lock().unwrap().execute("DELETE FROM library WHERE id = ?1", [id])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_roundtrip_and_library_search() {
        let s = Store::memory().unwrap();
        assert!(s.load_settings().unwrap().is_none());
        let st = Settings { download_dir: "D:/x".into(), max_concurrent: 2, max_segments: 6, notifications: false, ytdlp_channel: Default::default(), sessions: Default::default() };
        s.save_settings(&st).unwrap();
        let loaded = s.load_settings().unwrap().unwrap();
        assert_eq!(loaded.max_segments, 6);
        assert!(!loaded.notifications);

        let p = Path::new("D:/x/a.mp4");
        let rows = [("Me at the zoo", "youtube", "video", 30), ("100% real_title", "youtube", "audio", 10), ("Other", "tiktok", "video", 20)];
        for (title, platform, kind, size) in rows {
            s.add(&NewLibraryItem { source_url: "https://y/1", platform, title, uploader: Some("jawed"), kind, option_label: "240p", container: "mp4", file_path: p, file_size: size, duration: Some(19.0) }).unwrap();
        }
        let q = |text: &str| LibraryQuery { text: text.into(), ..Default::default() };
        assert_eq!(s.list(&q(""), 10).unwrap().len(), 3);
        assert_eq!(s.list(&q("zoo"), 10).unwrap().len(), 1);
        assert_eq!(s.list(&q("100%"), 10).unwrap().len(), 1);
        assert_eq!(s.list(&q("l_t"), 10).unwrap().len(), 1);
        assert_eq!(s.list(&q("JAWED"), 10).unwrap().len(), 3);

        let filtered = s.list(&LibraryQuery { platform: Some("youtube".into()), kind: Some("video".into()), ..Default::default() }, 10).unwrap();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].title, "Me at the zoo");
        let largest = s.list(&LibraryQuery { sort: Some("largest".into()), ..Default::default() }, 10).unwrap();
        assert_eq!(largest.iter().map(|i| i.file_size).collect::<Vec<_>>(), vec![30, 20, 10]);
        let by_title = s.list(&LibraryQuery { sort: Some("title".into()), ..Default::default() }, 10).unwrap();
        assert_eq!(by_title[0].title, "100% real_title");
        // An unknown sort value falls back to newest-first rather than reaching SQL.
        assert!(s.list(&LibraryQuery { sort: Some("title; DROP TABLE library".into()), ..Default::default() }, 10).is_ok());

        let f = s.facets().unwrap();
        assert_eq!(f.total, 3);
        assert_eq!(f.platforms[0].value, "youtube");
        assert_eq!(f.platforms[0].count, 2);

        let first = s.list(&q(""), 1).unwrap()[0].id;
        s.remove(first).unwrap();
        assert_eq!(s.list(&q(""), 10).unwrap().len(), 2);
    }

    #[test]
    fn older_settings_json_gets_defaults() {
        let s = Store::memory().unwrap();
        s.conn.lock().unwrap().execute("INSERT INTO settings VALUES('app', '{\"downloadDir\":\"D:/x\",\"maxConcurrent\":3,\"maxSegments\":8}')", []).unwrap();
        assert!(s.load_settings().unwrap().unwrap().notifications);
    }

    #[test]
    fn tasks_roundtrip() {
        let s = Store::memory().unwrap();
        s.save_task("b", "{\"n\":2}", 2).unwrap();
        s.save_task("a", "{\"n\":1}", 1).unwrap();
        s.save_task("b", "{\"n\":3}", 2).unwrap();
        assert_eq!(s.load_tasks().unwrap(), vec!["{\"n\":1}".to_string(), "{\"n\":3}".to_string()]);
        s.delete_task("a").unwrap();
        assert_eq!(s.load_tasks().unwrap().len(), 1);
    }
}
