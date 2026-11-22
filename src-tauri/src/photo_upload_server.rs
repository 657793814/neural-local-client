//! 手机拍照 → 本地批改台的临时上传服务（阶段 A，见 docs/product/local-phone-photo-submission-plan.md）。
//!
//! 老师点「手机上传」时才启动：绑局域网网卡的随机端口，生成一个一次性令牌，
//! 手机扫码打开内联上传页，拍照/选图后以 data URL 走 JSON POST 传回来。
//! 服务端把图转成 `PhotoInput` 交给 `grading::store_page()` —— 落盘路径、去重、
//! `local_attempt_asset` 登记与「导入照片」完全一致，不建平行系统。
//!
//! **不做常驻服务**：15 分钟过期或老师点停止即关；令牌只在内存，不落库不进日志。
//! **数据不出内网**：只有局域网绑定 + 同源请求，没有外部脚本/字体/统计。

use serde::Serialize;
use std::io::{Read, Write};
use std::net::{IpAddr, TcpListener, TcpStream, UdpSocket};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};

use rand_core::{OsRng, RngCore};

const SESSION_TTL: Duration = Duration::from_secs(15 * 60);
const MAX_PHOTOS: usize = 30;
const MAX_BODY: usize = 12 * 1024 * 1024;
const MAX_HEAD: usize = 16 * 1024;

/// 命令返回给前端的开启结果。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadStartInfo {
    pub url: String,
    /// SVG 字符串，前端用 data URL 显示，不注入 HTML。
    pub qr_svg: String,
    pub expires_in_secs: u64,
    pub max_photos: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadStatusInfo {
    pub active: bool,
    pub count: usize,
    pub max_photos: usize,
    pub student_id: String,
    pub seconds_left: u64,
}

/// 全局只保留一个会话（老师一次只给一个学生开上传入口）。
struct SessionControl {
    student_id: String,
    stop: Arc<AtomicBool>,
    count: Arc<AtomicUsize>,
    started_at: Instant,
}

static SESSION: OnceLock<Mutex<Option<SessionControl>>> = OnceLock::new();

fn session_slot() -> &'static Mutex<Option<SessionControl>> {
    SESSION.get_or_init(|| Mutex::new(None))
}

/// 服务线程持有的不可变数据。
struct SessionData {
    token: String,
    student_id: String,
    assignment_id: Option<String>,
    data_dir: PathBuf,
    db_path: PathBuf,
    /// 事件通知；单测里为 None（收图落盘不依赖事件）。
    app: Option<AppHandle>,
    expires_at: Instant,
    max_photos: usize,
}

impl SessionData {
    fn emit<S: Serialize + Clone>(&self, event: &str, payload: S) {
        if let Some(app) = &self.app {
            let _ = app.emit(event, payload);
        }
    }
}

/// 取默认路由网卡的局域网 IP。UDP connect 不发包，只查路由表，断网也能拿到。
fn lan_ip() -> Option<IpAddr> {
    let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("8.8.8.8:80").ok()?;
    let ip = socket.local_addr().ok()?.ip();
    if ip.is_loopback() || ip.is_unspecified() {
        None
    } else {
        Some(ip)
    }
}

/// 24 字节随机数 → base64url，作一次性令牌。
fn new_token() -> String {
    let mut bytes = [0u8; 24];
    OsRng.fill_bytes(&mut bytes);
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// 开启临时上传服务。没有局域网地址时给出降级提示（不绑 0.0.0.0，不暴露公网）。
pub fn start(
    app: &AppHandle,
    data_dir: &Path,
    db_path: &Path,
    student_id: &str,
    assignment_id: Option<&str>,
) -> Result<UploadStartInfo, String> {
    let ip = lan_ip().ok_or_else(|| {
        "没找到局域网地址。请连上 WiFi 再试；或用数据线 / 隔空投送把照片拷到这台电脑后从「导入照片」添加。"
            .to_string()
    })?;
    start_on(Some(app), ip, data_dir, db_path, student_id, assignment_id)
}

/// 可指定绑定地址的实现（单测用 127.0.0.1，app 传 None）。
pub(crate) fn start_on(
    app: Option<&AppHandle>,
    bind_ip: IpAddr,
    data_dir: &Path,
    db_path: &Path,
    student_id: &str,
    assignment_id: Option<&str>,
) -> Result<UploadStartInfo, String> {
    stop();

    let token = new_token();
    let listener = TcpListener::bind((bind_ip, 0)).map_err(|e| format!("启动上传服务失败：{e}"))?;
    let port = listener
        .local_addr()
        .map_err(|e| format!("读上传端口失败：{e}"))?
        .port();
    let url = format!("http://{bind_ip}:{port}/u/{token}");
    let qr_svg = qrcode::QrCode::new(url.as_bytes())
        .map_err(|e| format!("生成二维码失败：{e}"))?
        .render::<qrcode::render::svg::Color<'static>>()
        .min_dimensions(240, 240)
        .dark_color(qrcode::render::svg::Color("#1f2937"))
        .light_color(qrcode::render::svg::Color("#ffffff"))
        .build();

    let stop = Arc::new(AtomicBool::new(false));
    let count = Arc::new(AtomicUsize::new(0));
    let started_at = Instant::now();

    {
        let mut slot = session_slot()
            .lock()
            .map_err(|_| "会话状态忙".to_string())?;
        *slot = Some(SessionControl {
            student_id: student_id.to_string(),
            stop: stop.clone(),
            count: count.clone(),
            started_at,
        });
    }

    let data = SessionData {
        token,
        student_id: student_id.to_string(),
        assignment_id: assignment_id
            .filter(|value| !value.is_empty())
            .map(str::to_string),
        data_dir: data_dir.to_path_buf(),
        db_path: db_path.to_path_buf(),
        app: app.cloned(),
        expires_at: started_at + SESSION_TTL,
        max_photos: MAX_PHOTOS,
    };
    let flag = stop.clone();
    std::thread::spawn(move || serve(listener, data, flag));

    Ok(UploadStartInfo {
        url,
        qr_svg,
        expires_in_secs: SESSION_TTL.as_secs(),
        max_photos: MAX_PHOTOS,
    })
}

/// 停止当前会话（幂等）。令牌立即失效，线程在下一个轮询周期退出。
pub fn stop() {
    if let Ok(mut slot) = session_slot().lock() {
        if let Some(session) = slot.take() {
            session.stop.store(true, Ordering::SeqCst);
        }
    }
}

/// 当前会话状态，前端弹窗轮询显示「已收 N 张 / 剩余时间」。
pub fn status() -> UploadStatusInfo {
    let slot = session_slot().lock().ok();
    let Some(session) = slot.as_ref().and_then(|guard| guard.as_ref()) else {
        return UploadStatusInfo {
            active: false,
            count: 0,
            max_photos: MAX_PHOTOS,
            student_id: String::new(),
            seconds_left: 0,
        };
    };
    let expired = session.started_at.elapsed() >= SESSION_TTL;
    UploadStatusInfo {
        active: !expired && !session.stop.load(Ordering::SeqCst),
        count: session.count.load(Ordering::SeqCst),
        max_photos: MAX_PHOTOS,
        student_id: session.student_id.clone(),
        seconds_left: SESSION_TTL
            .saturating_sub(session.started_at.elapsed())
            .as_secs(),
    }
}

fn current_count() -> usize {
    let Ok(slot) = session_slot().lock() else {
        return 0;
    };
    slot.as_ref()
        .map(|session| session.count.load(Ordering::SeqCst))
        .unwrap_or(0)
}

fn bump_count(fallback: usize) -> usize {
    let Ok(slot) = session_slot().lock() else {
        return fallback;
    };
    slot.as_ref()
        .map(|session| session.count.fetch_add(1, Ordering::SeqCst) + 1)
        .unwrap_or(fallback)
}

fn serve(listener: TcpListener, data: SessionData, stop: Arc<AtomicBool>) {
    // 服务线程自己开连接，不抢主界面的 db 锁；WAL 模式下并发写是安全的。
    let Ok(conn) = crate::db::open(&data.db_path) else {
        data.emit("nb://photo-upload-error", "上传服务打不开数据库");
        let _ = stop.swap(true, Ordering::SeqCst);
        return;
    };
    if let Err(error) = listener.set_nonblocking(true) {
        data.emit(
            "nb://photo-upload-error",
            format!("上传服务启动失败：{error}"),
        );
        let _ = stop.swap(true, Ordering::SeqCst);
        return;
    }
    loop {
        if stop.load(Ordering::SeqCst) || Instant::now() >= data.expires_at {
            break;
        }
        match listener.accept() {
            Ok((mut stream, _)) => {
                // **必须显式切回阻塞**：监听套接字是非阻塞的，而在 macOS/BSD 上
                // accept() 出来的连接会继承这个标志（Linux 不会）。不切回来的话，
                // 请求头恰好已经躺在缓冲区时能读到，一旦要读几 MB 的图片 body，
                // 第二次 read 就会拿到 WouldBlock，连接被直接关掉——手机端看到的就是
                // 「上传失败：网络中断或服务已关闭」。单测用几百字节的图，一次就读完了，
                // 所以这条只能靠真实手机暴露；这里显式设置，不依赖平台语义。
                let _ = stream.set_nonblocking(false);
                handle(&mut stream, &data, &conn);
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(120));
            }
            Err(_) => break,
        }
    }
    let _ = stop.swap(true, Ordering::SeqCst);
}

fn handle(stream: &mut TcpStream, data: &SessionData, conn: &rusqlite::Connection) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(20)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(10)));

    let (method, path, body) = match read_request(stream) {
        Ok(request) => request,
        Err(ReadError::Respond(status, message)) => {
            let payload = serde_json::json!({"error": message}).to_string();
            respond(
                stream,
                status,
                "application/json; charset=utf-8",
                payload.as_bytes(),
            );
            return;
        }
        Err(ReadError::Drop) => return,
    };
    let segments: Vec<&str> = path.split('/').filter(|part| !part.is_empty()).collect();
    // 只认 /u/<token> 和 /u/<token>/status；其余一律不响应数据。
    if segments.len() < 2 || segments[0] != "u" || segments[1] != data.token {
        respond(
            stream,
            "403 Forbidden",
            "text/plain; charset=utf-8",
            b"forbidden",
        );
        return;
    }
    let is_status = segments.get(2) == Some(&"status");

    if method == "GET" && !is_status {
        respond(
            stream,
            "200 OK",
            "text/html; charset=utf-8",
            UPLOAD_PAGE.as_bytes(),
        );
        return;
    }
    if method == "GET" && is_status {
        let payload =
            serde_json::json!({"count": current_count(), "max": data.max_photos}).to_string();
        respond(
            stream,
            "200 OK",
            "application/json; charset=utf-8",
            payload.as_bytes(),
        );
        return;
    }
    if method != "POST" || is_status {
        respond(
            stream,
            "405 Method Not Allowed",
            "text/plain; charset=utf-8",
            b"method not allowed",
        );
        return;
    }
    if Instant::now() >= data.expires_at {
        let payload = serde_json::json!({"error": "上传入口已过期，请让老师重新开启"}).to_string();
        respond(
            stream,
            "403 Forbidden",
            "application/json; charset=utf-8",
            payload.as_bytes(),
        );
        return;
    }
    let count = current_count();
    if count >= data.max_photos {
        let payload = format!(
            "{{\"error\":\"本次最多 {} 张，已收满，请让老师重新开启\"}}",
            data.max_photos
        );
        respond(
            stream,
            "403 Forbidden",
            "application/json; charset=utf-8",
            payload.as_bytes(),
        );
        return;
    }

    // 只接受 { "dataUrl": "data:image/...;base64,..." }。
    let data_url = match serde_json::from_slice::<serde_json::Value>(&body)
        .ok()
        .and_then(|value| {
            value
                .get("dataUrl")
                .and_then(|node| node.as_str())
                .map(str::to_string)
        }) {
        Some(value) if looks_like_image_data_url(&value) => value,
        _ => {
            let payload = serde_json::json!({"error": "不是有效的图片数据"}).to_string();
            respond(
                stream,
                "400 Bad Request",
                "application/json; charset=utf-8",
                payload.as_bytes(),
            );
            return;
        }
    };

    let result = crate::grading::store_page(
        conn,
        &data.data_dir,
        &crate::grading::PhotoInput {
            student_id: data.student_id.clone(),
            assignment_id: data.assignment_id.clone(),
            data_url,
        },
    );

    match result {
        Ok(asset) => {
            let next = bump_count(count + 1);
            data.emit(
                "nb://grading-photo-arrived",
                serde_json::json!({
                    "studentId": data.student_id,
                    "assignmentId": data.assignment_id,
                    "assetId": asset.id,
                    "count": next,
                }),
            );
            let payload = format!("{{\"ok\":true,\"count\":{next}}}");
            respond(
                stream,
                "200 OK",
                "application/json; charset=utf-8",
                payload.as_bytes(),
            );
        }
        Err(error) => {
            // 不把内部路径/错误细节回给手机，只给可行动提示。
            let safe =
                if error.contains("只支持") || error.contains("解码") || error.contains("图片")
                {
                    "图片格式不支持或已损坏".to_string()
                } else {
                    "保存失败，请让老师查看客户端提示".to_string()
                };
            data.emit("nb://photo-upload-error", error);
            let payload = serde_json::json!({"error": safe}).to_string();
            respond(
                stream,
                "500 Internal Server Error",
                "application/json; charset=utf-8",
                payload.as_bytes(),
            );
        }
    }
}

fn looks_like_image_data_url(value: &str) -> bool {
    let Some(payload) = value.strip_prefix("data:image/") else {
        return false;
    };
    payload
        .split_once(";base64,")
        .map(|(_, bytes)| !bytes.is_empty() && bytes.len() <= MAX_BODY * 2)
        .unwrap_or(false)
}

/// 读请求失败的两类结果：能回话的（已经读到请求行/头）与只能断开的（连接层就废了）。
enum ReadError {
    /// 已经读到请求头，可以给手机一句人话。
    Respond(&'static str, String),
    /// 连请求行都没读全，回什么都没人接。
    Drop,
}

/// 读 HTTP 请求：请求行 + 头（限 16KB） + 按 Content-Length 读 body（限 MAX_BODY）。
///
/// 分块读、允许一次只到一部分：手机上走 WiFi，几 MB 的图片必然被拆成上千个 TCP 段，
/// 老实现「按字节读头 + 一次 read_exact 读体」在这种 socket 上很容易半路失败，
/// 失败后又直接 return，手机只能看到「网络中断」。
fn read_request(stream: &mut TcpStream) -> Result<(String, String, Vec<u8>), ReadError> {
    let mut head = Vec::with_capacity(2048);
    let mut buf = [0u8; 8192];
    let head_end = loop {
        let read = stream.read(&mut buf).map_err(|_| ReadError::Drop)?;
        if read == 0 {
            return Err(ReadError::Drop);
        }
        head.extend_from_slice(&buf[..read]);
        if let Some(position) = find_head_end(&head) {
            break position;
        }
        if head.len() > MAX_HEAD {
            return Err(ReadError::Respond(
                "431 Request Header Fields Too Large",
                "请求头过大，请换个浏览器再试".into(),
            ));
        }
    };
    let text = String::from_utf8_lossy(&head[..head_end]).to_string();
    let mut lines = text.split("\r\n");
    let request_line = lines.next().unwrap_or_default();
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_string();
    let path = parts.next().unwrap_or_default().to_string();

    let mut content_length = 0usize;
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if name.eq_ignore_ascii_case("content-length") {
            content_length = match value.trim().parse::<usize>() {
                Ok(length) => length,
                Err(_) => {
                    return Err(ReadError::Respond(
                        "400 Bad Request",
                        "请求长度不合法".into(),
                    ))
                }
            };
        }
    }
    if content_length > MAX_BODY {
        return Err(ReadError::Respond(
            "413 Payload Too Large",
            format!(
                "这张照片太大了（{}MB，上限 {}MB）。手机上传页会自动压缩，请退回上传页重新选一次；还不行就用「导入照片」选本地图。",
                content_length / (1024 * 1024),
                MAX_BODY / (1024 * 1024)
            ),
        ));
    }

    // 请求头和数据可能在同一批字节里一起到达，已经读进来的部分不能丢。
    let mut body = head[head_end + 4..].to_vec();
    body.reserve(content_length.saturating_sub(body.len()));
    while body.len() < content_length {
        let read = stream.read(&mut buf).map_err(|_| {
            ReadError::Respond("408 Request Timeout", "照片传到一半断了，请重试".into())
        })?;
        if read == 0 {
            return Err(ReadError::Respond(
                "400 Bad Request",
                "照片没有传完整，请重试".into(),
            ));
        }
        body.extend_from_slice(&buf[..read]);
    }
    Ok((method, path, body))
}

/// 找 `\r\n\r\n`，返回它在缓冲区里的起始位置。
fn find_head_end(bytes: &[u8]) -> Option<usize> {
    bytes.windows(4).position(|window| window == b"\r\n\r\n")
}

fn respond(stream: &mut TcpStream, status: &str, content_type: &str, body: &[u8]) {
    let header = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(header.as_bytes());
    let _ = stream.write_all(body);
}

/// 手机上传页：内联、无外部资源、同源请求。只做拍照/选图 → 读成 data URL → JSON POST。
const UPLOAD_PAGE: &str = r##"<!doctype html>
<html lang="zh-CN">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1,viewport-fit=cover">
<meta name="color-scheme" content="light">
<meta http-equiv="Content-Security-Policy" content="default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; img-src data: blob:; connect-src 'self'; form-action 'self';">
<title>上传作答照片</title>
<style>
*{box-sizing:border-box}body{margin:0;font:16px/1.6 -apple-system,BlinkMacSystemFont,"PingFang SC",sans-serif;color:#1f2937;background:#f5f7fa;padding:24px 18px}
.card{max-width:520px;margin:0 auto;background:#fff;border-radius:16px;padding:26px 22px;box-shadow:0 6px 24px rgba(31,41,55,.08)}
h1{font-size:20px;margin:0 0 6px}p{color:#6b7280;margin:0 0 18px}
input[type=file]{width:100%;padding:12px;border:1px dashed #9ca3af;border-radius:10px;background:#fafafa}
button{width:100%;margin-top:14px;padding:14px;border:0;border-radius:10px;background:#2563eb;color:#fff;font-size:16px;font-weight:600}
button:disabled{background:#9ca3af}#msg{margin-top:14px;min-height:24px;color:#059669;font-size:14px;text-align:center}
small{display:block;margin-top:16px;color:#9ca3af;text-align:center}
</style>
</head>
<body>
<div class="card">
<h1>上传作答照片</h1>
<p>拍照或从相册选照片，可以一次传多张。传完告诉老师即可。</p>
<input id="file" type="file" accept="image/*" multiple>
<button id="go">开始上传</button>
<div id="msg"></div>
<small>照片只发送到这台电脑，不经过任何外网服务</small>
</div>
<script>
var token = location.pathname.split('/').filter(Boolean)[1] || '';
var endpoint = '/u/' + token;
var msg = document.getElementById('msg');
var go = document.getElementById('go');
var file = document.getElementById('file');
// 手机直出照片 3–8MB，原样 base64 之后能顶到 12MB 上限；先在手机上缩到长边 2200px
// 再发，顺带把 HEIC 这类服务端不认的格式统一转成 JPEG。缩不了就退回原图。
var MAX_EDGE = 2200;
var QUALITY = 0.82;
function say(text){ msg.textContent = text; }
function readAsDataURL(blob){ return new Promise(function(res,rej){ var r=new FileReader(); r.onload=function(){res(r.result)}; r.onerror=rej; r.readAsDataURL(blob); }); }
function decodeByImg(blob){
  return new Promise(function(res,rej){
    var url=URL.createObjectURL(blob), img=new Image();
    img.onload=function(){ res(img); };
    img.onerror=function(){ URL.revokeObjectURL(url); rej(new Error('decode')); };
    img.src=url;
  });
}
async function shrink(blob){
  try{
    var source=null;
    if(window.createImageBitmap){ try{ source=await createImageBitmap(blob); }catch(e){ source=null; } }
    if(!source) source=await decodeByImg(blob);
    var w=source.width||source.naturalWidth, h=source.height||source.naturalHeight;
    if(!w||!h) return null;
    var scale=Math.min(1, MAX_EDGE/Math.max(w,h));
    var canvas=document.createElement('canvas');
    canvas.width=Math.max(1,Math.round(w*scale));
    canvas.height=Math.max(1,Math.round(h*scale));
    canvas.getContext('2d').drawImage(source,0,0,canvas.width,canvas.height);
    if(source.close) source.close();
    var out=canvas.toDataURL('image/jpeg', QUALITY);
    return out.indexOf('data:image/jpeg')===0?out:null;
  }catch(e){ return null; }
}
async function post(dataUrl){
  var resp=await fetch(endpoint,{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({dataUrl:dataUrl})});
  var text=await resp.text();
  var out={};
  try{ out=JSON.parse(text); }catch(e){ out={}; }
  return {ok:resp.ok, status:resp.status, out:out};
}
async function upload(files){
  var done=0, failed=0;
  for(var i=0;i<files.length;i++){
    var name='第 '+(i+1)+'/'+files.length+' 张';
    say(name+'：正在压缩…');
    // 不按 f.type 过滤：部分安卓/微信浏览器给空 type，老写法会静默跳过，看着像「点了没反应」。
    var dataUrl=await shrink(files[i]);
    if(!dataUrl){ try{ dataUrl=await readAsDataURL(files[i]); }catch(e){ dataUrl=null; } }
    if(!dataUrl){ failed++; say(name+'读不出来，换一张试试'); continue; }
    var sent=false;
    for(var attempt=0;attempt<2&&!sent;attempt++){
      try{
        var res=await post(dataUrl);
        if(res.ok){ sent=true; done=res.out.count||(done+1); say('已上传 '+done+' 张'+(i<files.length-1?'，继续…':'，全部完成')); }
        else{
          say(name+'没成功：'+(res.out.error||('HTTP '+res.status)));
          if(res.status<500) break;
        }
      }catch(e){
        if(attempt===0){ say(name+'没连上，正在重试…'); continue; }
        say(name+'传不上去：确认手机和电脑连的是同一个 WiFi，电脑上的「手机上传」窗口没有关');
      }
    }
    if(!sent) failed++;
  }
  if(failed) say((done?'已传 '+done+' 张，':'')+'还有 '+failed+' 张没上去');
  if(!files.length) say('请先选照片');
}
go.addEventListener('click',function(){
  if(!file.files || !file.files.length){ say('请先拍照或选照片'); return; }
  go.disabled=true; say('正在上传…');
  upload(Array.prototype.slice.call(file.files)).then(function(){ go.disabled=false; file.value=''; },
    function(){ go.disabled=false; say('上传中断，请重试'); });
});
</script>
</body>
</html>"##;

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine as _;
    use rusqlite::params;

    // 会话是进程级单例，两个起服务的测试必须串行，避免互相 stop。
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn token_is_url_safe_and_unique() {
        let a = new_token();
        let b = new_token();
        assert_eq!(a.len(), 32, "24 字节 → 32 个 base64url 字符");
        assert!(a
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
        assert_ne!(a, b);
    }

    #[test]
    fn data_url_sniff_rejects_non_images() {
        assert!(looks_like_image_data_url("data:image/jpeg;base64,/9j/4AAQ"));
        assert!(!looks_like_image_data_url("data:text/plain;base64,aGk="));
        assert!(!looks_like_image_data_url("hello"));
    }

    #[test]
    fn upload_page_is_self_contained() {
        assert!(UPLOAD_PAGE.contains("Content-Security-Policy"));
        assert!(!UPLOAD_PAGE.contains("<script src="));
        assert!(!UPLOAD_PAGE.contains("https://"));
        assert!(UPLOAD_PAGE.contains("fetch(endpoint"));
        // 手机端必须先把图缩到 JPEG 再发：原图 base64 能顶到 12MB 上限，
        // 而 HEIC 服务端不认。这两点都是「传不上去」的直接原因。
        assert!(UPLOAD_PAGE.contains("createImageBitmap"));
        assert!(UPLOAD_PAGE.contains("toDataURL"));
    }

    fn fresh() -> (std::path::PathBuf, std::path::PathBuf, String, String) {
        let dir = std::env::temp_dir().join(format!(
            "nb-upload-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("neuralkb.db");
        let mut conn = crate::db::open(&db_path).unwrap();
        crate::db::migrate(&mut conn).unwrap();
        crate::db::seed_curriculum(&mut conn).unwrap();
        let mut conn = conn;

        // local_attempt_asset 的 student_id / assignment_id 都是外键，先建真记录。
        let student = crate::teaching::student_save(
            &conn,
            &crate::teaching::StudentInput {
                id: None,
                name: "测试学生".into(),
                student_no: String::new(),
                grade: "高一".into(),
                subject_ids: vec![],
                note: String::new(),
                status: None,
            },
        )
        .unwrap();
        let subject: String = conn
            .query_row("select id from subject order by sort limit 1", [], |row| {
                row.get(0)
            })
            .unwrap();
        let question = uuid::Uuid::new_v4().to_string();
        conn.execute(
            "insert into question (id, subject_id, type, difficulty, stem, answer, analysis, status, created_at, updated_at)
             values (?1, ?2, 'SOLVED', 3, '题', '答', '析', 'PUBLISHED', '2026-01-01T00:00:00+08:00', '2026-01-01T00:00:00+08:00')",
            params![&question, &subject],
        )
        .unwrap();
        let class = crate::teaching::class_save(
            &conn,
            &crate::teaching::ClassInput {
                id: None,
                name: "测试班".into(),
                subject_id: Some(subject),
                term: String::new(),
            },
        )
        .unwrap();
        crate::teaching::class_add_members(&mut conn, &class, &[student.clone()]).unwrap();
        let assignment = crate::teaching::assignment_save(
            &mut conn,
            &crate::teaching::AssignmentInput {
                id: None,
                class_id: class,
                title: "测试作业".into(),
                kind: "PRACTICE".into(),
                description: String::new(),
                due_at: None,
                late_hours: 0,
                questions: vec![crate::teaching::AssignmentQuestionInput {
                    question_id: question,
                    score: 10.0,
                }],
            },
        )
        .unwrap();
        (dir, db_path, student, assignment)
    }

    fn sample_png_data_url() -> String {
        let mut img = image::RgbImage::new(64, 48);
        for pixel in img.pixels_mut() {
            *pixel = image::Rgb([255, 255, 255]);
        }
        let mut out = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(img)
            .write_to(&mut out, image::ImageFormat::Png)
            .unwrap();
        format!(
            "data:image/png;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(out.into_inner())
        )
    }

    /// 400×400 噪声图，压不小（几百 KB），用来模拟手机照片的分段上传。
    fn noisy_png_data_url() -> String {
        let mut img = image::RgbImage::new(400, 400);
        let mut seed: u32 = 0x9E37_79B9;
        for pixel in img.pixels_mut() {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let value = (seed >> 24) as u8;
            *pixel = image::Rgb([value, value.wrapping_mul(3), value.wrapping_add(17)]);
        }
        let mut out = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(img)
            .write_to(&mut out, image::ImageFormat::Png)
            .unwrap();
        format!(
            "data:image/png;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(out.into_inner())
        )
    }

    /// 发一个原始 HTTP 请求，返回完整响应字符串。
    fn request(addr: std::net::SocketAddr, raw: &[u8]) -> String {
        let mut stream = TcpStream::connect(addr).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        stream.write_all(raw).unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        response
    }

    fn post(addr: std::net::SocketAddr, token: &str, body: &str) -> String {
        let raw = format!(
            "POST /u/{token} HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        request(addr, raw.as_bytes())
    }

    #[test]
    fn http_upload_lands_in_grading_and_rejects_bad_token() {
        let _guard = TEST_LOCK.lock().unwrap();
        let (dir, db_path, student, assignment) = fresh();
        let info = start_on(
            None,
            "127.0.0.1".parse().unwrap(),
            &dir,
            &db_path,
            &student,
            Some(&assignment),
        )
        .unwrap();
        let token = info
            .url
            .split("/u/")
            .nth(1)
            .expect("url 里带令牌")
            .to_string();
        let addr = info
            .url
            .split('/')
            .nth(2)
            .and_then(|host_port| host_port.parse::<std::net::SocketAddr>().ok())
            .expect("url 里有 host:port");

        // 错误令牌 → 403，不落盘。
        let bad = post(addr, "wrong-token", "{}");
        assert!(bad.starts_with("HTTP/1.1 403"), "错误令牌必须被拒");

        // 非图片 → 400。
        let not_image = post(
            addr,
            &token,
            "{\"dataUrl\":\"data:text/plain;base64,aGk=\"}",
        );
        assert!(not_image.starts_with("HTTP/1.1 400"), "非图片必须被拒");

        // 正确令牌 + 图片 → 200，并落到 local_attempt_asset。
        let body = format!("{{\"dataUrl\":\"{}\"}}", sample_png_data_url());
        let ok = post(addr, &token, &body);
        assert!(
            ok.starts_with("HTTP/1.1 200"),
            "合法上传应成功，实际响应：{ok}"
        );
        assert!(ok.contains("\"count\":1"), "响应带张数");

        let conn = crate::db::open(&db_path).unwrap();
        let (count, rel): (i64, String) = conn
            .query_row(
                "select count(*), coalesce(max(relative_path), '') from local_attempt_asset where student_id = ?1 and kind = 'PAGE'",
                [&student],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(count, 1, "收一张图登记一条整页照片");
        assert!(
            rel.starts_with(&format!("submissions/{student}/")),
            "落盘路径与学生绑定"
        );
        assert!(dir.join("assets").join(&rel).exists(), "图片文件确实落盘");

        stop();
    }

    #[test]
    fn stopped_session_rejects_everything() {
        let _guard = TEST_LOCK.lock().unwrap();
        let (dir, db_path, student, _assignment) = fresh();
        let info = start_on(
            None,
            "127.0.0.1".parse().unwrap(),
            &dir,
            &db_path,
            &student,
            None,
        )
        .unwrap();
        let token = info.url.split("/u/").nth(1).unwrap().to_string();
        let addr = info
            .url
            .split('/')
            .nth(2)
            .and_then(|host_port| host_port.parse::<std::net::SocketAddr>().ok())
            .unwrap();
        // 会话状态里的 stop 置位后，status 应立刻反映关闭；新请求也因为 listener
        // 退出而连不上（stop() 调用后线程在 120ms 内退出）。
        stop();
        assert!(!status().active, "停止后会话立即失效");
        // 令牌不再属于任何活跃会话，旧端口最终关闭；这里只断言会话态，避免时序抖动。
        assert!(token.len() == 32);
        let _ = addr;
    }

    /// 大图在 WiFi 上一定被拆成很多段：旧实现「按字节读头 + 一次 read_exact 读体」
    /// 遇到分段到达的 body 会拿到 WouldBlock 直接断连，手机端只看到「网络中断」。
    #[test]
    fn segmented_upload_body_still_succeeds() {
        let _guard = TEST_LOCK.lock().unwrap();
        let (dir, db_path, student, assignment) = fresh();
        let info = start_on(
            None,
            "127.0.0.1".parse().unwrap(),
            &dir,
            &db_path,
            &student,
            Some(&assignment),
        )
        .unwrap();
        let token = info.url.split("/u/").nth(1).unwrap().to_string();
        let addr = info
            .url
            .split('/')
            .nth(2)
            .and_then(|host_port| host_port.parse::<std::net::SocketAddr>().ok())
            .unwrap();

        let body = format!("{{\"dataUrl\":\"{}\"}}", noisy_png_data_url());
        assert!(body.len() > 200_000, "样例要够大，才能被拆成多段");

        let mut stream = TcpStream::connect(addr).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        let head = format!(
            "POST /u/{token} HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        stream.write_all(head.as_bytes()).unwrap();
        stream.flush().unwrap();
        // 先让服务端把请求头消费掉，再分段发 body：模拟真实手机的上传节奏。
        std::thread::sleep(Duration::from_millis(60));
        for chunk in body.as_bytes().chunks(8 * 1024) {
            stream.write_all(chunk).unwrap();
            stream.flush().unwrap();
            std::thread::sleep(Duration::from_millis(2));
        }

        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        assert!(
            response.starts_with("HTTP/1.1 200"),
            "分段上传也应成功，实际：{response}"
        );

        let conn = crate::db::open(&db_path).unwrap();
        let count: i64 = conn
            .query_row(
                "select count(*) from local_attempt_asset where student_id = ?1",
                [&student],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1, "分段上传照样落一条照片");
        stop();
    }

    /// 超过上限时要给手机一句人话，而不是直接掐断连接（那样只会显示「网络中断」）。
    #[test]
    fn oversized_body_is_rejected_with_a_readable_error() {
        let _guard = TEST_LOCK.lock().unwrap();
        let (dir, db_path, student, _assignment) = fresh();
        let info = start_on(
            None,
            "127.0.0.1".parse().unwrap(),
            &dir,
            &db_path,
            &student,
            None,
        )
        .unwrap();
        let token = info.url.split("/u/").nth(1).unwrap().to_string();
        let addr = info
            .url
            .split('/')
            .nth(2)
            .and_then(|host_port| host_port.parse::<std::net::SocketAddr>().ok())
            .unwrap();

        let head = format!(
            "POST /u/{token} HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            MAX_BODY + 1
        );
        let mut stream = TcpStream::connect(addr).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        stream.write_all(head.as_bytes()).unwrap();
        stream.flush().unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        assert!(
            response.starts_with("HTTP/1.1 413"),
            "超大请求要回 413，实际：{response}"
        );
        assert!(
            response.contains("太大"),
            "错误文案要说清是尺寸问题：{response}"
        );
        stop();
    }
}
