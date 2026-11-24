use crate::error::{AppError, AppResult};
use crate::paths::AppPaths;
use crate::sidecars::{manifest::SidecarId, prepare_verified_executable};
use crate::store::Database;
use crate::tiktok::{url_decode, url_encode};
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use tokio::process::Command;
use uuid::Uuid;

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

static MEDIA_SERVER: OnceLock<(u16, String)> = OnceLock::new();
static STREAM_URL_CACHE: OnceLock<Mutex<HashMap<String, (Instant, Vec<String>)>>> = OnceLock::new();

fn stream_url_cache() -> &'static Mutex<HashMap<String, (Instant, Vec<String>)>> {
    STREAM_URL_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn ensure_media_server() -> AppResult<(u16, String)> {
    if let Some(existing) = MEDIA_SERVER.get() {
        return Ok(existing.clone());
    }

    let listener = TcpListener::bind("127.0.0.1:0")
        .map_err(|e| AppError::Network(format!("Serveur média local impossible : {e}")))?;
    let port = listener
        .local_addr()
        .map_err(|e| AppError::Network(format!("Port média local introuvable : {e}")))?
        .port();
    let token = Uuid::new_v4().to_string().replace('-', "");

    let expected_token = token.clone();
    std::thread::spawn(move || {
        for incoming in listener.incoming() {
            if let Ok(stream) = incoming {
                let tok = expected_token.clone();
                std::thread::spawn(move || {
                    let _ = handle_media_connection(stream, &tok);
                });
            }
        }
    });

    let _ = MEDIA_SERVER.set((port, token.clone()));
    Ok(MEDIA_SERVER.get().cloned().unwrap_or((port, token)))
}

pub fn build_local_media_url(path: &Path) -> AppResult<String> {
    let (port, token) = ensure_media_server()?;
    let encoded_path = url_encode(&path.to_string_lossy());
    Ok(format!(
        "http://127.0.0.1:{port}/media?token={token}&path={encoded_path}"
    ))
}

async fn resolve_preview_stream_urls(
    db: &Database,
    paths: &AppPaths,
    workspace_dir: &Path,
    source_url: &str,
    force_refresh: bool,
) -> AppResult<Vec<String>> {
    if !force_refresh {
        if let Ok(guard) = stream_url_cache().lock() {
            if let Some((ts, urls)) = guard.get(source_url) {
                if ts.elapsed() < Duration::from_secs(20 * 60) && !urls.is_empty() {
                    return Ok(urls.clone());
                }
            }
        }
    }

    let yt_dlp = prepare_verified_executable(db, paths, SidecarId::YtDlp).await?;
    let mut cmd = Command::new(&yt_dlp);
    cmd.env("PYTHONUTF8", "1").args([
        "--force-ipv4",
        "--retries",
        "4",
        "--socket-timeout",
        "15",
        "-f",
        "480p/360p/best[height<=480]/bestvideo[height<=480]+bestaudio/worst",
        "-g",
        "--no-playlist",
    ]);
    let cookie_file = workspace_dir.join("cookies.txt");
    if cookie_file.is_file() {
        cmd.arg("--cookies").arg(&cookie_file);
    }
    cmd.arg(source_url);
    #[cfg(target_os = "windows")]
    cmd.creation_flags(CREATE_NO_WINDOW);

    let out = tokio::time::timeout(Duration::from_secs(25), cmd.output())
        .await
        .map_err(|_| AppError::Network("Délai dépassé lors de la résolution du flux aperçu.".to_string()))?
        .map_err(|e| AppError::Sidecar(format!("Impossible de lancer yt-dlp : {e}")))?;

    if !out.status.success() {
        return Err(AppError::Network("Résolution du flux 480p/360p échouée.".to_string()));
    }

    let urls: Vec<String> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::trim)
        .filter(|l| l.starts_with("http"))
        .take(2)
        .map(ToOwned::to_owned)
        .collect();

    if urls.is_empty() {
        return Err(AppError::Network("Aucune URL de flux vidéo résolue.".to_string()));
    }

    if let Ok(mut guard) = stream_url_cache().lock() {
        guard.insert(source_url.to_string(), (Instant::now(), urls.clone()));
    }

    Ok(urls)
}

pub async fn build_fast_stream_preview_mp4(
    db: &Database,
    paths: &AppPaths,
    workspace_dir: &Path,
    source_url: &str,
    clip_id: &str,
    clip_start_sec: f64,
    clip_dur_sec: f64,
) -> AppResult<PathBuf> {
    let prev_dir = workspace_dir.join("previews");
    std::fs::create_dir_all(&prev_dir)?;

    let s_tag = (clip_start_sec.max(0.0) * 10.0).round() as u64;
    let d_tag = (clip_dur_sec.max(1.0) * 10.0).round() as u64;
    let out_mp4 = prev_dir.join(format!("fast_{clip_id}_{s_tag}_{d_tag}.mp4"));

    if let Ok(meta) = std::fs::metadata(&out_mp4) {
        if meta.len() > 8_192 {
            return Ok(out_mp4);
        }
    }

    let ffmpeg = prepare_verified_executable(db, paths, SidecarId::Ffmpeg).await?;

    for attempt in 0..2u8 {
        let urls = resolve_preview_stream_urls(db, paths, workspace_dir, source_url, attempt > 0).await?;

        let mut cmd = Command::new(&ffmpeg);
        cmd.args([
            "-y",
            "-nostdin",
            "-hide_banner",
            "-loglevel",
            "error",
            "-ss",
            &format!("{:.2}", clip_start_sec.max(0.0)),
            "-i",
            &urls[0],
        ]);
        if urls.len() >= 2 {
            cmd.args([
                "-ss",
                &format!("{:.2}", clip_start_sec.max(0.0)),
                "-i",
                &urls[1],
                "-map",
                "0:v:0",
                "-map",
                "1:a:0",
                "-shortest",
            ]);
        }
        cmd.args([
            "-t",
            &format!("{:.2}", clip_dur_sec.max(1.0)),
            "-vf",
            "scale='min(480,iw)':-2",
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-crf",
            "28",
            "-pix_fmt",
            "yuv420p",
            "-c:a",
            "aac",
            "-b:a",
            "96k",
            "-ar",
            "48000",
            "-movflags",
            "+faststart",
        ])
        .arg(&out_mp4);

        #[cfg(target_os = "windows")]
        cmd.creation_flags(CREATE_NO_WINDOW);

        let res = tokio::time::timeout(Duration::from_secs(45), cmd.output()).await;
        if let Ok(Ok(output)) = res {
            if output.status.success() {
                if let Ok(meta) = std::fs::metadata(&out_mp4) {
                    if meta.len() > 8_192 {
                        return Ok(out_mp4);
                    }
                }
            }
        }
        let _ = std::fs::remove_file(&out_mp4);
        if let Ok(mut guard) = stream_url_cache().lock() {
            guard.remove(source_url);
        }
    }

    Err(AppError::Sidecar("Aperçu rapide depuis le flux 480p indisponible.".to_string()))
}

pub async fn build_webview_preview_mp4(
    db: &Database,
    paths: &AppPaths,
    seg_path: &Path,
    workspace_dir: &Path,
    clip_id: &str,
    trim_start_sec: f64,
    clip_dur_sec: f64,
) -> AppResult<PathBuf> {
    let prev_dir = workspace_dir.join("previews");
    std::fs::create_dir_all(&prev_dir)?;

    let s_tag = (trim_start_sec.max(0.0) * 10.0).round() as u64;
    let d_tag = (clip_dur_sec.max(1.0) * 10.0).round() as u64;
    let out_mp4 = prev_dir.join(format!("wv_{clip_id}_{s_tag}_{d_tag}.mp4"));

    if let Ok(meta) = std::fs::metadata(&out_mp4) {
        if meta.len() > 8_192 {
            return Ok(out_mp4);
        }
    }

    let ffmpeg = prepare_verified_executable(db, paths, SidecarId::Ffmpeg).await?;
    let mut cmd = Command::new(&ffmpeg);
    cmd.args([
        "-y",
        "-nostdin",
        "-hide_banner",
        "-loglevel",
        "error",
        "-ss",
        &format!("{:.3}", trim_start_sec.max(0.0)),
        "-t",
        &format!("{:.3}", clip_dur_sec.max(1.0)),
        "-i",
    ])
    .arg(seg_path)
    .args([
        "-vf",
        "scale='min(480,iw)':-2",
        "-c:v",
        "libx264",
        "-preset",
        "ultrafast",
        "-crf",
        "28",
        "-pix_fmt",
        "yuv420p",
        "-c:a",
        "aac",
        "-b:a",
        "96k",
        "-ar",
        "48000",
        "-movflags",
        "+faststart",
    ])
    .arg(&out_mp4);

    #[cfg(target_os = "windows")]
    cmd.creation_flags(CREATE_NO_WINDOW);

    let res = cmd
        .output()
        .await
        .map_err(|e| AppError::Sidecar(format!("Aperçu FFmpeg impossible : {e}")))?;

    if !res.status.success() || !out_mp4.is_file() {
        let _ = std::fs::remove_file(&out_mp4);
        let stderr = String::from_utf8_lossy(&res.stderr);
        return Err(AppError::Sidecar(format!(
            "Conversion aperçu WebView2 échouée : {}",
            stderr.lines().last().unwrap_or("erreur inconnue")
        )));
    }

    Ok(out_mp4)
}

fn handle_media_connection(mut stream: TcpStream, expected_token: &str) -> std::io::Result<()> {
    let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(5)));
    let reader_stream = stream.try_clone()?;
    let mut reader = BufReader::new(reader_stream);

    let mut request_line = String::new();
    if reader.read_line(&mut request_line)? == 0 {
        return Ok(());
    }

    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("GET");
    let target = parts.next().unwrap_or("/");

    let mut range_header: Option<String> = None;
    loop {
        let mut line = String::new();
        let n = reader.read_line(&mut line)?;
        if n == 0 || line == "\r\n" || line == "\n" {
            break;
        }
        if let Some((name, val)) = line.split_once(':') {
            if name.trim().eq_ignore_ascii_case("range") {
                range_header = Some(val.trim().to_string());
            }
        }
    }

    if method == "OPTIONS" {
        stream.write_all(
            b"HTTP/1.1 204 No Content\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: GET, HEAD, OPTIONS\r\nAccess-Control-Allow-Headers: Range\r\nConnection: close\r\n\r\n",
        )?;
        return Ok(());
    }

    if method != "GET" && method != "HEAD" {
        stream.write_all(b"HTTP/1.1 405 Method Not Allowed\r\nConnection: close\r\n\r\n")?;
        return Ok(());
    }

    let Some((_, query)) = target.split_once('?') else {
        stream.write_all(b"HTTP/1.1 400 Bad Request\r\nConnection: close\r\n\r\n")?;
        return Ok(());
    };

    let mut req_token = None;
    let mut req_path = None;
    for pair in query.split('&') {
        if let Some((k, v)) = pair.split_once('=') {
            match k {
                "token" => req_token = Some(url_decode(v)),
                "path" => req_path = Some(url_decode(v)),
                _ => {}
            }
        }
    }

    if req_token.as_deref() != Some(expected_token) {
        stream.write_all(b"HTTP/1.1 403 Forbidden\r\nConnection: close\r\n\r\n")?;
        return Ok(());
    }

    let Some(file_path_str) = req_path else {
        stream.write_all(b"HTTP/1.1 400 Bad Request\r\nConnection: close\r\n\r\n")?;
        return Ok(());
    };

    let path = Path::new(&file_path_str);
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_default();
    let content_type = match ext.as_str() {
        "mp4" | "m4v" => "video/mp4",
        "webm" => "video/webm",
        "wav" => "audio/wav",
        "mp3" => "audio/mpeg",
        "m4a" => "audio/mp4",
        _ => {
            stream.write_all(b"HTTP/1.1 403 Forbidden\r\nConnection: close\r\n\r\n")?;
            return Ok(());
        }
    };

    let mut file = match File::open(path) {
        Ok(f) => f,
        Err(_) => {
            stream.write_all(b"HTTP/1.1 404 Not Found\r\nConnection: close\r\n\r\n")?;
            return Ok(());
        }
    };

    let file_len = file.metadata()?.len();
    if file_len == 0 {
        stream.write_all(b"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n")?;
        return Ok(());
    }

    if let Some(range_val) = range_header {
        if let Some((start, end)) = parse_byte_range(&range_val, file_len) {
            let chunk_len = end - start + 1;
            let header = format!(
                "HTTP/1.1 206 Partial Content\r\nContent-Type: {content_type}\r\nAccept-Ranges: bytes\r\nContent-Range: bytes {start}-{end}/{file_len}\r\nContent-Length: {chunk_len}\r\nAccess-Control-Allow-Origin: *\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n"
            );
            stream.write_all(header.as_bytes())?;
            if method == "GET" {
                file.seek(SeekFrom::Start(start))?;
                let mut limited = file.take(chunk_len);
                let _ = std::io::copy(&mut limited, &mut stream);
            }
            return Ok(());
        } else {
            let resp = format!(
                "HTTP/1.1 416 Range Not Satisfiable\r\nContent-Range: bytes */{file_len}\r\nConnection: close\r\n\r\n"
            );
            stream.write_all(resp.as_bytes())?;
            return Ok(());
        }
    }

    let header = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nAccept-Ranges: bytes\r\nContent-Length: {file_len}\r\nAccess-Control-Allow-Origin: *\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n"
    );
    stream.write_all(header.as_bytes())?;
    if method == "GET" {
        let _ = std::io::copy(&mut file, &mut stream);
    }
    Ok(())
}

pub fn parse_byte_range(header: &str, file_len: u64) -> Option<(u64, u64)> {
    let spec = header.trim().strip_prefix("bytes=")?;
    let first_range = spec.split(',').next()?.trim();
    let (start_s, end_s) = first_range.split_once('-')?;

    if start_s.is_empty() {
        let suffix: u64 = end_s.trim().parse().ok()?;
        if suffix == 0 {
            return None;
        }
        let start = file_len.saturating_sub(suffix);
        return Some((start, file_len - 1));
    }

    let start: u64 = start_s.trim().parse().ok()?;
    if start >= file_len {
        return None;
    }
    let end = if end_s.trim().is_empty() {
        file_len - 1
    } else {
        end_s.trim().parse::<u64>().ok()?.min(file_len - 1)
    };
    if end < start {
        return None;
    }
    Some((start, end))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_http_byte_ranges() {
        assert_eq!(parse_byte_range("bytes=0-1023", 5000), Some((0, 1023)));
        assert_eq!(parse_byte_range("bytes=100-", 5000), Some((100, 4999)));
        assert_eq!(parse_byte_range("bytes=-500", 5000), Some((4500, 4999)));
        assert_eq!(parse_byte_range("bytes=6000-7000", 5000), None);
    }
}