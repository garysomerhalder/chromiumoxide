//! A fake adb server for the Android tests: the adb host protocol on a TCP
//! port, one device (`emulator-5554`, model "Fake Pixel"), a few shell
//! commands, `sync:` pushes, package installs, and `localabstract:` DevTools
//! sockets backed by local headless Chromium instances (one for Chrome,
//! started by `am start`, one for a WebView of `com.example.app`, pid 4242).

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

pub const SERIAL: &str = "emulator-5554";
pub const WEBVIEW_SOCKET: &str = "webview_devtools_remote_4242";

/// A headless Chromium with a DevTools port, standing in for a device app.
struct DevtoolsApp {
    child: Child,
    port: u16,
    ws_path: String,
    dir: PathBuf,
}

impl Drop for DevtoolsApp {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn start_app(name: &str) -> DevtoolsApp {
    let exe = chromiumoxide::Chromium::executable_path(&chromiumoxide::LaunchOptions::default())
        .expect("a Chromium for the fake device")
        .executable;
    let dir = std::env::temp_dir().join(format!("cox-fake-adb-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let child = Command::new(exe)
        .args(["--headless", "--no-sandbox", "--remote-debugging-port=0"])
        .arg(format!("--user-data-dir={}", dir.display()))
        .arg("about:blank")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    for _ in 0..500 {
        if let Ok(text) = std::fs::read_to_string(dir.join("DevToolsActivePort")) {
            let mut lines = text.lines();
            if let (Some(port), Some(path)) = (lines.next(), lines.next()) {
                return DevtoolsApp {
                    child,
                    port: port.parse().unwrap(),
                    ws_path: path.to_owned(),
                    dir,
                };
            }
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("fake device app {name} did not start");
}

#[derive(Default)]
struct State {
    /// Pushed files, by device path.
    files: HashMap<String, Vec<u8>>,
    /// Installed APK sizes.
    installs: Vec<usize>,
    /// Every shell command received.
    shell_log: Vec<String>,
    /// DevTools sockets: name -> app.
    sockets: HashMap<String, Arc<DevtoolsApp>>,
}

/// The fake adb server; dropped with the test.
pub struct FakeAdb {
    pub port: u16,
    state: Arc<Mutex<State>>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for FakeAdb {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl FakeAdb {
    pub async fn start() -> FakeAdb {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let state = Arc::new(Mutex::new(State::default()));
        let webview = Arc::new(start_app("webview"));
        state
            .lock()
            .unwrap()
            .sockets
            .insert(WEBVIEW_SOCKET.into(), webview);
        let st = state.clone();
        let task = tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                tokio::spawn(serve(stream, st.clone()));
            }
        });
        FakeAdb { port, state, task }
    }

    pub fn file(&self, path: &str) -> Option<Vec<u8>> {
        self.state.lock().unwrap().files.get(path).cloned()
    }

    pub fn installs(&self) -> Vec<usize> {
        self.state.lock().unwrap().installs.clone()
    }

    pub fn shell_log(&self) -> Vec<String> {
        self.state.lock().unwrap().shell_log.clone()
    }
}

async fn read_request(stream: &mut TcpStream) -> Option<String> {
    let mut len = [0u8; 4];
    stream.read_exact(&mut len).await.ok()?;
    let len = usize::from_str_radix(std::str::from_utf8(&len).ok()?, 16).ok()?;
    let mut msg = vec![0u8; len];
    stream.read_exact(&mut msg).await.ok()?;
    String::from_utf8(msg).ok()
}

async fn okay_with_payload(stream: &mut TcpStream, payload: &[u8]) {
    let _ = stream.write_all(b"OKAY").await;
    let _ = stream
        .write_all(format!("{:04x}", payload.len()).as_bytes())
        .await;
    let _ = stream.write_all(payload).await;
}

async fn fail(stream: &mut TcpStream, msg: &str) {
    let _ = stream.write_all(b"FAIL").await;
    let _ = stream
        .write_all(format!("{:04x}{msg}", msg.len()).as_bytes())
        .await;
}

/// A 1x1 PNG, the "screenshot".
const PNG: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f, 0x15, 0xc4,
    0x89, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0xf8, 0xcf, 0xc0, 0xf0,
    0x1f, 0x00, 0x05, 0x00, 0x01, 0xff, 0x89, 0x99, 0x3d, 0x1d, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45,
    0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
];

async fn serve(mut stream: TcpStream, state: Arc<Mutex<State>>) {
    let Some(mut request) = read_request(&mut stream).await else {
        return;
    };
    if request == "host:devices" || request == "host:devices-l" {
        okay_with_payload(&mut stream, format!("{SERIAL}\tdevice\n").as_bytes()).await;
        return;
    }
    if let Some(serial) = request.strip_prefix("host:transport:") {
        if serial != SERIAL {
            fail(&mut stream, "device not found").await;
            return;
        }
        let _ = stream.write_all(b"OKAY").await;
        let Some(next) = read_request(&mut stream).await else {
            return;
        };
        request = next;
    }
    if let Some(cmd) = request.strip_prefix("shell:") {
        shell(stream, cmd.to_owned(), state).await;
    } else if request == "sync:" {
        let _ = stream.write_all(b"OKAY").await;
        sync(stream, state).await;
    } else if let Some(name) = request.strip_prefix("localabstract:") {
        let app = state.lock().unwrap().sockets.get(name).cloned();
        match app {
            Some(app) => {
                let _ = stream.write_all(b"OKAY").await;
                proxy_devtools(stream, app).await;
            }
            None => fail(&mut stream, "connection refused").await,
        }
    } else {
        fail(&mut stream, &format!("unknown service {request}")).await;
    }
}

async fn shell(mut stream: TcpStream, cmd: String, state: Arc<Mutex<State>>) {
    state.lock().unwrap().shell_log.push(cmd.clone());
    let _ = stream.write_all(b"OKAY").await;
    let out: Vec<u8> = if cmd == "getprop ro.product.model" {
        b"Fake Pixel\n".to_vec()
    } else if cmd == "screencap -p" {
        PNG.to_vec()
    } else if cmd == "cat" {
        // Echoes until the client closes (`device.open("shell:cat")`).
        let mut buf = [0u8; 4096];
        while let Ok(n) = stream.read(&mut buf).await {
            if n == 0 || stream.write_all(&buf[..n]).await.is_err() {
                break;
            }
        }
        return;
    } else if let Some(rest) = cmd.strip_prefix("echo ") {
        if let Some((data, file)) = rest.split_once(" | base64 -d > ") {
            use base64::Engine;
            let decoded = base64::engine::general_purpose::STANDARD
                .decode(data.trim_matches('"'))
                .unwrap_or_default();
            state.lock().unwrap().files.insert(file.into(), decoded);
            Vec::new()
        } else {
            format!("{rest}\n").into_bytes()
        }
    } else if cmd.contains("/proc/net/unix") {
        let names: Vec<String> = state
            .lock()
            .unwrap()
            .sockets
            .keys()
            .filter(|n| n.contains("webview_devtools_remote"))
            .cloned()
            .collect();
        names
            .iter()
            .map(|n| format!("0000000000000000: 00000002 00000000 00010000 0001 01 12345 @{n}\n"))
            .collect::<String>()
            .into_bytes()
    } else if cmd.starts_with("ps -A | grep 4242") {
        b"u0_a123       4242   100 1000000  50000 0                   0 S com.example.app\n"
            .to_vec()
    } else if cmd.starts_with("am start") && cmd.contains("com.android.chrome") {
        let line = state
            .lock()
            .unwrap()
            .files
            .get("/data/local/tmp/chrome-command-line")
            .map(|b| String::from_utf8_lossy(b).into_owned())
            .unwrap_or_default();
        let socket = line
            .split_whitespace()
            .find_map(|a| a.strip_prefix("--remote-debugging-socket-name="))
            .unwrap_or("chrome_devtools_remote")
            .to_owned();
        let app = Arc::new(start_app("chrome"));
        state.lock().unwrap().sockets.insert(socket, app);
        b"Starting: Intent { act=android.intent.action.VIEW }\n".to_vec()
    } else if let Some(args) = cmd.strip_prefix("cmd package install ") {
        let size: usize = args
            .split_whitespace()
            .last()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        let mut apk = vec![0u8; size];
        let _ = stream.read_exact(&mut apk).await;
        state.lock().unwrap().installs.push(size);
        b"Success\n".to_vec()
    } else if let Some(path) = cmd.strip_prefix("rm ") {
        state.lock().unwrap().files.remove(path.trim());
        Vec::new()
    } else {
        // am force-stop, grep without match, ...
        Vec::new()
    };
    let _ = stream.write_all(&out).await;
    let _ = stream.shutdown().await;
}

/// `sync:` with SEND / DATA / DONE, answered with OKAY.
async fn sync(mut stream: TcpStream, state: Arc<Mutex<State>>) {
    let mut path = String::new();
    let mut content = Vec::new();
    loop {
        let mut head = [0u8; 8];
        if stream.read_exact(&mut head).await.is_err() {
            return;
        }
        let id = &head[..4];
        let len = u32::from_le_bytes(head[4..].try_into().unwrap()) as usize;
        match id {
            b"SEND" => {
                let mut spec = vec![0u8; len];
                let _ = stream.read_exact(&mut spec).await;
                let spec = String::from_utf8_lossy(&spec).into_owned();
                path = spec
                    .rsplit_once(',')
                    .map_or(spec.clone(), |(p, _)| p.to_owned());
            }
            b"DATA" => {
                let mut chunk = vec![0u8; len];
                let _ = stream.read_exact(&mut chunk).await;
                content.extend(chunk);
            }
            b"DONE" => {
                state
                    .lock()
                    .unwrap()
                    .files
                    .insert(path.clone(), content.clone());
                let _ = stream.write_all(b"OKAY\0\0\0\0").await;
                return;
            }
            _ => return,
        }
    }
}

/// Forwards a `localabstract:` DevTools socket to the app's port, pointing
/// the WebSocket upgrade for `/devtools/browser` at the app's real path.
async fn proxy_devtools(mut client: TcpStream, app: Arc<DevtoolsApp>) {
    let Ok(mut upstream) = TcpStream::connect(("127.0.0.1", app.port)).await else {
        return;
    };
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        if client.read_exact(&mut byte).await.is_err() {
            return;
        }
        head.push(byte[0]);
    }
    let head = String::from_utf8_lossy(&head).replacen(
        "GET /devtools/browser ",
        &format!("GET {} ", app.ws_path),
        1,
    );
    if upstream.write_all(head.as_bytes()).await.is_err() {
        return;
    }
    let _ = tokio::io::copy_bidirectional(&mut client, &mut upstream).await;
}
