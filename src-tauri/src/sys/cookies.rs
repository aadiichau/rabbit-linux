//! Opt-in signed-in sessions (docs/DESIGN_SIGNED_IN_SESSIONS.md). Reads the user's own cookies for
//! one site from a Firefox profile or a Netscape cookies.txt file. Only cookies of that site's
//! domains are ever read into memory; values are never logged, persisted or sent anywhere else.

use std::fmt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};

/// Sites that can use a signed-in session. Each has its own switch in Settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Site {
    Bilibili,
    Douyin,
}

impl Site {
    /// Cookie domains read for this site (and their subdomains). Nothing else is read.
    pub fn domains(self) -> &'static [&'static str] {
        match self {
            Site::Bilibili => &["bilibili.com"],
            Site::Douyin => &["douyin.com"],
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Site::Bilibili => "Bilibili",
            Site::Douyin => "Douyin",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum CookieSource {
    /// A Firefox profile folder (the one containing `cookies.sqlite`).
    FirefoxProfile { path: PathBuf },
    /// A Netscape-format cookies.txt exported by the user.
    NetscapeFile { path: PathBuf },
}

/// What Settings stores: which source and which sites — never cookie values.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Sessions {
    pub enabled: bool,
    pub source: Option<CookieSource>,
    pub bilibili: bool,
    pub douyin: bool,
}

impl Sessions {
    /// The source to read for `site`, when the master switch and the site's switch are both on.
    pub fn source_for(&self, site: Site) -> Option<&CookieSource> {
        let on = self.enabled
            && match site {
                Site::Bilibili => self.bilibili,
                Site::Douyin => self.douyin,
            };
        if on { self.source.as_ref() } else { None }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct Cookie {
    /// As stored: `.bilibili.com` (domain cookie) or `www.bilibili.com` (host-only).
    pub domain: String,
    pub name: String,
    pub value: String,
    pub path: String,
    pub secure: bool,
    pub http_only: bool,
    /// Unix seconds; `None` for session cookies.
    pub expires: Option<i64>,
}

/// Values are secrets: Debug output never shows them.
impl fmt::Debug for Cookie {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Cookie({} {}=<redacted>)", self.domain, self.name)
    }
}

/// Whether a cookie domain belongs to one of `domains` (the domain itself or a subdomain).
fn in_domains(cookie_domain: &str, domains: &[&str]) -> bool {
    let d = cookie_domain.trim_start_matches('.').to_ascii_lowercase();
    domains.iter().any(|want| d == *want || d.strip_suffix(want).is_some_and(|rest| rest.ends_with('.')))
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

fn keep(c: &Cookie, domains: &[&str]) -> bool {
    in_domains(&c.domain, domains) && c.expires.is_none_or(|e| e > now()) && !c.name.is_empty()
}

/// Reads `site`'s cookies from `source`. Other domains are skipped while reading.
pub fn read(source: &CookieSource, site: Site) -> AppResult<Vec<Cookie>> {
    let cookies = match source {
        CookieSource::NetscapeFile { path } => {
            let text = std::fs::read_to_string(path).map_err(|e| AppError::Io(format!("couldn't read the cookies file ({e})")))?;
            parse_netscape(&text, site.domains())
        }
        CookieSource::FirefoxProfile { path } => read_firefox(path, site.domains())?,
    };
    Ok(cookies.into_iter().filter(|c| keep(c, site.domains())).collect())
}

/// Netscape cookies.txt: `domain  flag  path  secure  expiry  name  value`, tab-separated;
/// `#HttpOnly_` marks HTTP-only cookies; other `#` lines are comments.
pub fn parse_netscape(text: &str, domains: &[&str]) -> Vec<Cookie> {
    text.lines()
        .filter_map(|line| {
            let line = line.trim_end_matches('\r');
            let (http_only, line) = match line.strip_prefix("#HttpOnly_") {
                Some(rest) => (true, rest),
                None if line.starts_with('#') || line.trim().is_empty() => return None,
                None => (false, line),
            };
            let f: Vec<&str> = line.split('\t').collect();
            if f.len() < 7 || !in_domains(f[0], domains) {
                return None;
            }
            let expires = f[4].trim().parse::<i64>().ok().filter(|e| *e > 0);
            Some(Cookie {
                domain: f[0].to_string(),
                name: f[5].to_string(),
                value: f[6..].join("\t"),
                path: f[2].to_string(),
                secure: f[3].eq_ignore_ascii_case("TRUE"),
                http_only,
                expires,
            })
        })
        .collect()
}

pub fn write_netscape(cookies: &[Cookie]) -> String {
    let mut out = String::from("# Netscape HTTP Cookie File\n# Written by Rabbit for one yt-dlp run; deleted afterwards.\n\n");
    for c in cookies {
        let prefix = if c.http_only { "#HttpOnly_" } else { "" };
        let sub = if c.domain.starts_with('.') { "TRUE" } else { "FALSE" };
        let secure = if c.secure { "TRUE" } else { "FALSE" };
        out.push_str(&format!("{prefix}{}\t{sub}\t{}\t{secure}\t{}\t{}\t{}\n", c.domain, c.path, c.expires.unwrap_or(0), c.name, c.value));
    }
    out
}

/// Firefox keeps cookies unencrypted in `cookies.sqlite` (WAL mode). The database and its WAL are
/// copied to a private temp folder first, so a running Firefox is neither blocked nor modified.
fn read_firefox(profile: &Path, domains: &[&str]) -> AppResult<Vec<Cookie>> {
    let db = profile.join("cookies.sqlite");
    if !db.is_file() {
        return Err(AppError::NotFound(format!("no cookies.sqlite in the Firefox profile {}", profile.display())));
    }
    let tmp = TempDir::new()?;
    let copy = tmp.path.join("cookies.sqlite");
    std::fs::copy(&db, &copy).map_err(|e| AppError::Io(format!("couldn't copy Firefox's cookie database ({e})")))?;
    let wal = profile.join("cookies.sqlite-wal");
    if wal.is_file() {
        let _ = std::fs::copy(&wal, tmp.path.join("cookies.sqlite-wal"));
    }
    let conn = rusqlite::Connection::open(&copy)?;
    // Default container only (no `userContextId`/private attributes), as a normal tab sees them.
    let mut stmt = conn.prepare("SELECT host, name, value, path, isSecure, isHttpOnly, expiry FROM moz_cookies WHERE originAttributes = ''")?;
    let rows = stmt.query_map([], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?, r.get::<_, i64>(4)?, r.get::<_, i64>(5)?, r.get::<_, i64>(6)?))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (host, name, value, path, secure, http_only, expiry) = row?;
        if !in_domains(&host, domains) {
            continue; // never kept in memory
        }
        // Firefox 133+ stores expiry in milliseconds; older versions in seconds.
        let expires = if expiry > 100_000_000_000 { expiry / 1000 } else { expiry };
        out.push(Cookie { domain: host, name, value, path, secure: secure != 0, http_only: http_only != 0, expires: (expires > 0).then_some(expires) });
    }
    Ok(out)
}

/// A private temporary folder, removed on drop.
struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new() -> AppResult<Self> {
        let n = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        let path = std::env::temp_dir().join(format!("rabbit-cookies-{}-{n}", std::process::id()));
        let mut builder = std::fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(&path)?;
        Ok(TempDir { path })
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// A cookies.txt written for one yt-dlp run, deleted when dropped (success, failure or cancel).
pub struct TempCookieFile {
    pub path: PathBuf,
}

impl TempCookieFile {
    pub fn create(dir: &Path, cookies: &[Cookie]) -> AppResult<Self> {
        std::fs::create_dir_all(dir)?;
        let n = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        let path = dir.join(format!("session-{n}.txt"));
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&path)?;
        std::io::Write::write_all(&mut file, write_netscape(cookies).as_bytes())?;
        Ok(TempCookieFile { path })
    }
}

impl Drop for TempCookieFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// `name=value; …` for a request header.
pub fn header(cookies: &[Cookie]) -> String {
    cookies.iter().map(|c| format!("{}={}", c.name, c.value)).collect::<Vec<_>>().join("; ")
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FirefoxProfile {
    pub name: String,
    pub path: PathBuf,
    pub is_default: bool,
    pub has_cookies: bool,
}

/// Profiles listed in Firefox's `profiles.ini` (classic install and the Microsoft Store package).
pub fn firefox_profiles() -> Vec<FirefoxProfile> {
    let mut roots = Vec::new();
    #[cfg(windows)]
    if let Some(appdata) = std::env::var_os("APPDATA") {
        roots.push(PathBuf::from(appdata).join("Mozilla").join("Firefox"));
    }
    #[cfg(windows)]
    if let Some(local) = std::env::var_os("LOCALAPPDATA")
        && let Ok(rd) = std::fs::read_dir(PathBuf::from(local).join("Packages"))
    {
        for e in rd.flatten().filter(|e| e.file_name().to_string_lossy().starts_with("Mozilla.Firefox")) {
            roots.push(e.path().join("LocalCache").join("Roaming").join("Mozilla").join("Firefox"));
        }
    }
    #[cfg(target_os = "linux")]
    if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home);
        roots.extend([
            home.join(".mozilla/firefox"),
            home.join(".var/app/org.mozilla.firefox/.mozilla/firefox"),
            home.join("snap/firefox/common/.mozilla/firefox"),
        ]);
        let config = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".config"));
        roots.push(config.join("mozilla/firefox"));
    }
    roots.iter().flat_map(|r| std::fs::read_to_string(r.join("profiles.ini")).map(|ini| parse_profiles_ini(&ini, r)).unwrap_or_default()).collect()
}

pub fn parse_profiles_ini(ini: &str, root: &Path) -> Vec<FirefoxProfile> {
    let mut out = Vec::new();
    let mut install_default: Vec<String> = Vec::new();
    let mut section = String::new();
    let (mut name, mut path, mut relative, mut default) = (None::<String>, None::<String>, true, false);
    let flush = |section: &str, name: &mut Option<String>, path: &mut Option<String>, relative: bool, default: bool, out: &mut Vec<FirefoxProfile>| {
        if section.starts_with("Profile")
            && let Some(p) = path.take()
        {
            let full = if relative { root.join(p.replace('\\', "/")) } else { PathBuf::from(&p) };
            let has_cookies = full.join("cookies.sqlite").is_file();
            out.push(FirefoxProfile { name: name.take().unwrap_or_else(|| p.clone()), path: full, is_default: default, has_cookies });
        }
        *name = None;
    };
    for line in ini.lines().map(str::trim) {
        if line.starts_with('[') && line.ends_with(']') {
            flush(&section, &mut name, &mut path, relative, default, &mut out);
            section = line[1..line.len() - 1].to_string();
            (relative, default) = (true, false);
            continue;
        }
        let Some((k, v)) = line.split_once('=') else { continue };
        match (section.starts_with("Install"), k) {
            (true, "Default") => install_default.push(v.to_string()),
            (false, "Name") => name = Some(v.to_string()),
            (false, "Path") => path = Some(v.to_string()),
            (false, "IsRelative") => relative = v == "1",
            (false, "Default") => default = v == "1",
            _ => {}
        }
    }
    flush(&section, &mut name, &mut path, relative, default, &mut out);
    // The [Install…] Default is the profile Firefox actually opens; prefer it as the default.
    if !install_default.is_empty() {
        for p in &mut out {
            p.is_default = install_default.iter().any(|d| p.path.ends_with(d.replace('\\', "/")));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const TXT: &str = "# Netscape HTTP Cookie File\n\
.bilibili.com\tTRUE\t/\tFALSE\t4102444800\tbuvid3\tabc\n\
#HttpOnly_.bilibili.com\tTRUE\t/\tTRUE\t4102444800\tSESSDATA\ts%2Cecret\n\
.google.com\tTRUE\t/\tTRUE\t4102444800\tSID\tgoogle-secret\n\
evilbilibili.com\tFALSE\t/\tFALSE\t4102444800\tx\ty\n\
www.douyin.com\tFALSE\t/\tFALSE\t0\tsessionid\tdy\n\
.bilibili.com\tTRUE\t/\tFALSE\t1000\told\texpired\n";

    #[test]
    fn netscape_reads_only_the_site() {
        let b = parse_netscape(TXT, Site::Bilibili.domains());
        let names: Vec<_> = b.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["buvid3", "SESSDATA", "old"], "google.com and look-alike domains never read");
        assert!(b[1].http_only && b[1].secure);
        let d = parse_netscape(TXT, Site::Douyin.domains());
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].expires, None, "0 = session cookie");
    }

    #[test]
    fn expired_cookies_are_dropped_and_values_redacted() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("c.txt");
        std::fs::write(&p, TXT).unwrap();
        let got = read(&CookieSource::NetscapeFile { path: p }, Site::Bilibili).unwrap();
        assert_eq!(got.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["buvid3", "SESSDATA"]);
        let dbg = format!("{got:?}");
        assert!(!dbg.contains("ecret") && !dbg.contains("abc"), "Debug must not show values: {dbg}");
    }

    #[test]
    fn netscape_round_trip_and_temp_file_cleanup() {
        let cookies = parse_netscape(TXT, Site::Bilibili.domains());
        let back = parse_netscape(&write_netscape(&cookies), Site::Bilibili.domains());
        assert_eq!(back, cookies);
        let dir = tempfile::tempdir().unwrap();
        let path = {
            let f = TempCookieFile::create(dir.path(), &cookies).unwrap();
            assert!(f.path.is_file());
            f.path.clone()
        };
        assert!(!path.exists(), "deleted on drop");
        assert_eq!(header(&cookies[..2]), "buvid3=abc; SESSDATA=s%2Cecret");
    }

    #[test]
    fn firefox_database_reader() {
        // A cookies.sqlite with Firefox's moz_cookies columns; one row per case.
        let dir = tempfile::tempdir().unwrap();
        let conn = rusqlite::Connection::open(dir.path().join("cookies.sqlite")).unwrap();
        conn.execute_batch(
            "CREATE TABLE moz_cookies (id INTEGER PRIMARY KEY, originAttributes TEXT NOT NULL DEFAULT '', name TEXT, value TEXT, host TEXT, path TEXT,
                expiry INTEGER, lastAccessed INTEGER, creationTime INTEGER, isSecure INTEGER, isHttpOnly INTEGER);
             INSERT INTO moz_cookies (originAttributes, name, value, host, path, expiry, isSecure, isHttpOnly) VALUES
               ('', 'SESSDATA', 'v1', '.bilibili.com', '/', 4102444800000, 1, 1),
               ('', 'bili_jct', 'v2', '.bilibili.com', '/', 4102444800, 0, 0),
               ('^userContextId=2', 'SESSDATA', 'container', '.bilibili.com', '/', 4102444800, 1, 1),
               ('', 'SID', 'google', '.google.com', '/', 4102444800, 1, 1);",
        )
        .unwrap();
        drop(conn);
        let got = read(&CookieSource::FirefoxProfile { path: dir.path().to_path_buf() }, Site::Bilibili).unwrap();
        assert_eq!(got.iter().map(|c| (c.name.as_str(), c.value.as_str())).collect::<Vec<_>>(), [("SESSDATA", "v1"), ("bili_jct", "v2")]);
        assert_eq!(got[0].expires, Some(4102444800), "millisecond expiry normalised");
    }

    #[test]
    fn switches_gate_the_source() {
        let src = CookieSource::NetscapeFile { path: "c.txt".into() };
        let mut s = Sessions { enabled: false, source: Some(src.clone()), bilibili: true, douyin: false };
        assert!(s.source_for(Site::Bilibili).is_none(), "master switch off");
        s.enabled = true;
        assert_eq!(s.source_for(Site::Bilibili), Some(&src));
        assert!(s.source_for(Site::Douyin).is_none());
        assert_eq!(Sessions::default(), Sessions { enabled: false, source: None, bilibili: false, douyin: false });
    }

    #[test]
    fn linux_relative_profiles_keep_directory_separators() {
        let root = tempfile::tempdir().unwrap();
        let profile = root.path().join("Profiles/default");
        std::fs::create_dir_all(&profile).unwrap();
        std::fs::write(profile.join("cookies.sqlite"), b"").unwrap();
        let p = parse_profiles_ini("[Install123]\nDefault=Profiles/default\n[Profile0]\nName=default\nIsRelative=1\nPath=Profiles/default\n", root.path());
        assert_eq!(p[0].path, profile);
        assert!(p[0].is_default);
        assert!(p[0].has_cookies);
    }

    #[test]
    fn profiles_ini() {
        let ini = "[Install308046B0AF4A39CB]\nDefault=Profiles/abc.default-release\n\n[Profile1]\nName=default\nIsRelative=1\nPath=Profiles/xyz.default\nDefault=1\n\n[Profile0]\nName=default-release\nIsRelative=1\nPath=Profiles/abc.default-release\n";
        let p = parse_profiles_ini(ini, Path::new("C:\\FF"));
        assert_eq!(p.len(), 2);
        let def: Vec<_> = p.iter().filter(|p| p.is_default).map(|p| p.name.as_str()).collect();
        assert_eq!(def, ["default-release"], "the install default wins");
        assert_eq!(p[1].path, Path::new("C:\\FF").join("Profiles/abc.default-release"));
    }
}
