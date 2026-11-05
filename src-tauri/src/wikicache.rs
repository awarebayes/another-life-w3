//! Disk cache for the wiki illustrations. The Fandom CDN answers in 1.5-24 s per image, so the app downloads
//! every linked image once (in the background, many at a time), keeps the files in the app's cache folder and
//! serves them to the webview through the `wikiimg` URI scheme. Only `https://static.wikia.nocookie.net/` URLs
//! are ever fetched.
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

pub const USER_AGENT: &str = "AnotherLife-fan-tool/0.1 (open source)";
const HOST_PREFIX: &str = "https://static.wikia.nocookie.net/";
const EXTS: [&str; 5] = ["jpg", "png", "webp", "gif", "bin"];
const MAX_BYTES: u64 = 32 * 1024 * 1024;

/// Only the wiki's image CDN, plain printable URLs.
pub fn allowed(url: &str) -> bool {
    url.starts_with(HOST_PREFIX) && url.len() > HOST_PREFIX.len() && url.len() < 2048 && !url.chars().any(|c| c.is_control() || c == ' ' || c == '"' || c == '\\')
}

/// The UI asks for 800 px; the questbook lists 1000 px. Both map to one cache entry.
pub fn normalize(url: &str) -> String {
    url.replace("/scale-to-width-down/1000", "/scale-to-width-down/800")
}

/// Decode the request path of a `wikiimg` URL (`/<percent-encoded original URL>`) and validate it.
pub fn url_from_request_path(path: &str) -> Result<String, String> {
    let raw = path.strip_prefix('/').unwrap_or(path);
    let url = percent_encoding::percent_decode_str(raw).decode_utf8().map_err(|_| "bad encoding".to_owned())?.into_owned();
    if allowed(&url) {
        Ok(normalize(&url))
    } else {
        Err("only static.wikia.nocookie.net images are served".into())
    }
}

/// FNV-1a, 64 bit, twice with different bases (128 bits of file name) - no dependency, plenty for a few hundred URLs.
fn hash_hex(s: &str) -> String {
    let mut a: u64 = 0xcbf29ce484222325;
    let mut b: u64 = 0x84222325cbf29ce4;
    for &byte in s.as_bytes() {
        a = (a ^ u64::from(byte)).wrapping_mul(0x100000001b3);
        b = (b ^ u64::from(byte)).wrapping_mul(0x100000001b3).rotate_left(5);
    }
    format!("{a:016x}{b:016x}")
}

fn ext_for(content_type: &str) -> &'static str {
    let ct = content_type.split(';').next().unwrap_or("").trim().to_ascii_lowercase();
    match ct.as_str() {
        "image/jpeg" | "image/jpg" => "jpg",
        "image/png" => "png",
        "image/webp" => "webp",
        "image/gif" => "gif",
        _ => "bin",
    }
}

pub fn mime_for(ext: &str) -> &'static str {
    match ext {
        "jpg" => "image/jpeg",
        "png" => "image/png",
        "webp" => "image/webp",
        "gif" => "image/gif",
        _ => "application/octet-stream",
    }
}

/// One shared agent (connection pool) for the whole process.
pub fn agent() -> ureq::Agent {
    static AGENT: std::sync::OnceLock<ureq::Agent> = std::sync::OnceLock::new();
    AGENT.get_or_init(|| ureq::Agent::config_builder().user_agent(USER_AGENT).timeout_global(Some(Duration::from_secs(60))).build().into()).clone()
}

/// The cached file for `url` (already normalized), if any.
pub fn cached_path(dir: &Path, url: &str) -> Option<(PathBuf, &'static str)> {
    let stem = hash_hex(url);
    EXTS.iter().find_map(|e| {
        let p = dir.join(format!("{stem}.{e}"));
        p.is_file().then_some((p, *e))
    })
}

static TMP_COUNTER: AtomicUsize = AtomicUsize::new(0);

/// Cached bytes and content type, downloading and storing (temp file + rename) on a miss.
pub fn get(dir: &Path, agent: &ureq::Agent, url: &str) -> Result<(Vec<u8>, &'static str), String> {
    let url = normalize(url);
    if !allowed(&url) {
        return Err("URL not allowed".into());
    }
    if let Some((p, ext)) = cached_path(dir, &url) {
        if let Ok(b) = std::fs::read(&p) {
            return Ok((b, mime_for(ext)));
        }
    }
    let resp = agent.get(&url).call().map_err(|e| format!("{url}: {e}"))?;
    let ctype = resp.headers().get("content-type").and_then(|v| v.to_str().ok()).unwrap_or("").to_owned();
    let ext = ext_for(&ctype);
    if ext == "bin" {
        return Err(format!("{url}: not an image ({ctype})"));
    }
    let bytes = resp.into_body().with_config().limit(MAX_BYTES).read_to_vec().map_err(|e| format!("{url}: {e}"))?;
    if bytes.is_empty() {
        return Err(format!("{url}: empty body"));
    }
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let stem = hash_hex(&url);
    let tmp = dir.join(format!("{stem}.{}.{}.tmp", std::process::id(), TMP_COUNTER.fetch_add(1, Ordering::Relaxed)));
    std::fs::write(&tmp, &bytes).map_err(|e| e.to_string())?;
    if let Err(e) = std::fs::rename(&tmp, dir.join(format!("{stem}.{ext}"))) {
        let _ = std::fs::remove_file(&tmp);
        // A concurrent download of the same URL may have won on Windows; the bytes in hand are still good.
        if cached_path(dir, &url).is_none() {
            return Err(e.to_string());
        }
    }
    Ok((bytes, mime_for(ext)))
}

/// Download every URL not yet cached with `workers` threads, in the given order. `progress(done, total)` is
/// called after each finished URL (cached ones count). Failures are skipped (the UI falls back to the CDN
/// through a normal request later).
pub fn prefetch(dir: &Path, urls: Vec<String>, workers: usize, progress: impl Fn(usize, usize) + Send + Sync + 'static) {
    let mut seen = std::collections::HashSet::new();
    let urls: Vec<String> = urls.into_iter().map(|u| normalize(&u)).filter(|u| allowed(u) && seen.insert(u.clone())).collect();
    let total = urls.len();
    let urls = Arc::new(urls);
    let next = Arc::new(AtomicUsize::new(0));
    let done = Arc::new(AtomicUsize::new(0));
    let progress = Arc::new(progress);
    let agent = agent();
    let handles: Vec<_> = (0..workers.max(1))
        .map(|_| {
            let (urls, next, done, progress, agent, dir) = (urls.clone(), next.clone(), done.clone(), progress.clone(), agent.clone(), dir.to_path_buf());
            std::thread::spawn(move || loop {
                let i = next.fetch_add(1, Ordering::Relaxed);
                let Some(u) = urls.get(i) else { break };
                if cached_path(&dir, u).is_none() {
                    let _ = get(&dir, &agent, u);
                }
                let d = done.fetch_add(1, Ordering::Relaxed) + 1;
                if d % 8 == 0 || d == total {
                    progress(d, total);
                }
            })
        })
        .collect();
    for h in handles {
        let _ = h.join();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_urls() {
        assert!(allowed("https://static.wikia.nocookie.net/witcher/images/8/8c/Griffinfield.jpg/revision/latest/scale-to-width-down/1000"));
        assert!(!allowed("http://static.wikia.nocookie.net/x.jpg"));
        assert!(!allowed("https://static.wikia.nocookie.net.evil.com/x"));
        assert!(!allowed("https://evil.com/https://static.wikia.nocookie.net/"));
        assert!(!allowed("https://static.wikia.nocookie.net/"));
        assert!(!allowed("file:///c:/x"));
        assert!(!allowed("https://static.wikia.nocookie.net/a b"));
    }

    #[test]
    fn request_path_round_trip() {
        let u = "https://static.wikia.nocookie.net/witcher/images/8/8c/A%20b.jpg/revision/latest/scale-to-width-down/1000?cb=1";
        let enc = percent_encoding::utf8_percent_encode(u, percent_encoding::NON_ALPHANUMERIC).to_string();
        let got = url_from_request_path(&format!("/{enc}")).unwrap();
        assert_eq!(got, u.replace("/1000", "/800"));
        assert!(url_from_request_path("/https%3A%2F%2Fevil.com%2Fx.png").is_err());
        assert!(url_from_request_path("/%ff%fe").is_err());
    }

    #[test]
    fn hash_and_ext() {
        assert_eq!(hash_hex("a").len(), 32);
        assert_ne!(hash_hex("a"), hash_hex("b"));
        assert_eq!(ext_for("image/jpeg; charset=x"), "jpg");
        assert_eq!(ext_for("text/html"), "bin");
    }

    /// Network check (skipped offline): two real images from images.toml into a temp cache; first download vs cached read.
    #[test]
    fn downloads_two_images() {
        let text = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../questbook/images.toml")).unwrap();
        let urls: Vec<String> = text.lines().filter_map(|l| l.strip_prefix("image = \"")).map(|l| l.trim_end_matches('"').to_owned()).take(2).collect();
        assert_eq!(urls.len(), 2);
        let dir = std::env::temp_dir().join(format!("w3al-wikicache-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let agent = agent();
        for u in &urls {
            let t = std::time::Instant::now();
            let (bytes, mime) = match get(&dir, &agent, u) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("offline, skipped: {e}");
                    return;
                }
            };
            let first = t.elapsed();
            assert!(bytes.len() > 1000 && mime.starts_with("image/"));
            let t = std::time::Instant::now();
            let (again, _) = get(&dir, &agent, u).unwrap();
            let second = t.elapsed();
            assert_eq!(bytes, again);
            println!("{} bytes: first {first:?}, cached {second:?}", bytes.len());
        }
        // Batch path with progress.
        let n = std::sync::Arc::new(AtomicUsize::new(0));
        let n2 = n.clone();
        let t = std::time::Instant::now();
        prefetch(&dir, urls, 4, move |d, _| n2.store(d, Ordering::Relaxed));
        println!("prefetch of 2 cached: {:?}", t.elapsed());
        assert_eq!(n.load(Ordering::Relaxed), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
