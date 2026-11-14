use crate::{
    capture,
    model::{self, Kind, Media, NameInfo, Settings},
};
use anyhow::{bail, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use chrono::Local;
use rand::{distributions::Alphanumeric, Rng};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, VecDeque},
    fs,
    io::{Read, Seek, SeekFrom},
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    process::{Child, Command},
    thread,
    time::{Duration, Instant},
};
use tungstenite::{client, Message, WebSocket};

const SCENE: &str = "Kuma Full Display";
const INPUT: &str = "Kuma Display";
pub const DESKTOP: &str = "Kuma System Audio";
pub const MIC: &str = "Kuma Microphone";

struct Socket {
    ws: WebSocket<TcpStream>,
    request: u64,
    events: VecDeque<Value>,
}

impl Socket {
    fn connect(port: u16, password: &str) -> Result<Self> {
        let tcp =
            TcpStream::connect_timeout(&([127, 0, 0, 1], port).into(), Duration::from_millis(250))?;
        tcp.set_read_timeout(Some(Duration::from_millis(200)))?;
        tcp.set_write_timeout(Some(Duration::from_secs(2)))?;
        let (ws, _) =
            client(format!("ws://127.0.0.1:{port}"), tcp).map_err(|e| anyhow::anyhow!("{e}"))?;
        let mut socket = Self {
            ws,
            request: 0,
            events: VecDeque::new(),
        };
        let hello = socket.next(Instant::now() + Duration::from_secs(2))?;
        let salt = hello["d"]["authentication"]["salt"]
            .as_str()
            .context("OBS did not request authentication")?;
        let challenge = hello["d"]["authentication"]["challenge"]
            .as_str()
            .context("OBS did not provide a challenge")?;
        let secret = STANDARD.encode(Sha256::digest(format!("{password}{salt}")));
        let auth = STANDARD.encode(Sha256::digest(format!("{secret}{challenge}")));
        socket.send(json!({"op":1,"d":{"rpcVersion":1,"eventSubscriptions":(1 << 6)|(1 << 16),"authentication":auth}}))?;
        let accepted = socket.next(Instant::now() + Duration::from_secs(2))?;
        if accepted["op"] != 2 {
            bail!("OBS did not accept authentication.");
        }
        Ok(socket)
    }

    fn send(&mut self, message: Value) -> Result<()> {
        self.ws.send(Message::Text(message.to_string().into()))?;
        Ok(())
    }

    fn poll(&mut self) -> Result<Option<Value>> {
        match self.ws.read() {
            Ok(Message::Text(text)) => Ok(Some(serde_json::from_str(text.as_str())?)),
            Ok(Message::Close(_)) => bail!("OBS disconnected."),
            Ok(_) => Ok(None),
            Err(tungstenite::Error::Io(error))
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                ) =>
            {
                Ok(None)
            }
            Err(error) => Err(error.into()),
        }
    }

    fn next(&mut self, deadline: Instant) -> Result<Value> {
        while Instant::now() < deadline {
            if let Some(event) = self.poll()? {
                return Ok(event);
            }
        }
        bail!("OBS did not respond in time.")
    }

    fn remember(&mut self, payload: Value) {
        if self.events.len() >= 100 {
            self.events.pop_front();
        }
        self.events.push_back(payload);
    }

    fn call(&mut self, name: &str, args: Value) -> Result<Value> {
        self.request += 1;
        let id = self.request.to_string();
        self.send(json!({"op":6,"d":{"requestType":name,"requestId":id,"requestData":args}}))?;
        let deadline = Instant::now() + Duration::from_secs(12);
        loop {
            let value = self.next(deadline)?;
            let body = &value["d"];
            if value["op"] == 5 {
                self.remember(body.clone());
            } else if value["op"] == 7 && body["requestId"] == id {
                if body["requestStatus"]["result"] == true {
                    return Ok(body["responseData"].clone());
                }
                let comment = body["requestStatus"]["comment"]
                    .as_str()
                    .unwrap_or("request rejected");
                bail!("{name}: {comment}");
            }
        }
    }

    fn call_simple(&mut self, name: &str) -> Result<Value> {
        self.call(name, json!({}))
    }

    fn event(&mut self, kind: &str, seconds: u64) -> Result<Value> {
        self.event_match(kind, seconds, |_| true)
    }

    fn event_match(
        &mut self,
        kind: &str,
        seconds: u64,
        predicate: impl Fn(&Value) -> bool,
    ) -> Result<Value> {
        let deadline = Instant::now() + Duration::from_secs(seconds);
        loop {
            if let Some(index) = self
                .events
                .iter()
                .position(|event| event["eventType"] == kind && predicate(&event["eventData"]))
            {
                return Ok(self.events.remove(index).unwrap_or_default()["eventData"].clone());
            }
            let event = self.next(deadline)?;
            if event["op"] == 5 {
                self.remember(event["d"].clone());
            }
        }
    }
}

pub struct Engine {
    root: PathBuf,
    child: Option<Child>,
    socket: Option<Socket>,
    display: Option<capture::Display>,
    dimensions: String,
    pub replay: Option<Instant>,
    pub recording: Option<Instant>,
    pub devices: [Vec<(String, String)>; 2],
    pub meters: [f32; 2],
    pub fallback: bool,
}

impl Engine {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            child: None,
            socket: None,
            display: None,
            dimensions: String::new(),
            replay: None,
            recording: None,
            devices: Default::default(),
            meters: [0.0; 2],
            fallback: false,
        }
    }

    fn socket(&mut self) -> Result<&mut Socket> {
        self.socket.as_mut().context("Capture engine is stopped.")
    }

    fn start_engine(&mut self, settings: &Settings, encoder: &str) -> Result<()> {
        self.stop();
        let display = capture::selected_display(&settings.monitor)?;
        let bin = self.root.join("obs/bin/64bit");
        let exe = bin.join("obs64.exe");
        if !exe.is_file() {
            bail!("The OBS capture runtime is missing. Extract the complete Windows package.");
        }
        let config = self.root.join("obs/config/obs-studio");
        let ws_dir = config.join("plugin_config/obs-websocket");
        let profiles = config.join("basic/profiles/Kuma");
        let scenes = config.join("basic/scenes");
        let scratch = config.join("kuma-media");
        for dir in [&ws_dir, &profiles, &scenes, &scratch] {
            fs::create_dir_all(dir)?;
        }
        let tcp = TcpListener::bind(("127.0.0.1", 0))?;
        let port = tcp.local_addr()?.port();
        let password: String = rand::thread_rng()
            .sample_iter(&Alphanumeric)
            .take(48)
            .map(char::from)
            .collect();
        fs::write(
            ws_dir.join("config.json"),
            serde_json::to_vec(&json!({
                "first_load":false,"server_enabled":true,"server_port":port,"alerts_enabled":false,
                "auth_required":true,"server_password":password
            }))?,
        )?;
        drop(tcp);
        fs::write(config.join("user.ini"), "[General]\nFirstRun=true\n[Basic]\nProfile=Kuma\nProfileDir=Kuma\nSceneCollection=Kuma\nSceneCollectionFile=Kuma\n[BasicWindow]\nSysTrayEnabled=true\nSysTrayWhenStarted=true\nHideOBSWindowsFromCapture=true\n")?;
        fs::write(
            config.join("global.ini"),
            "[General]\nEnableAutoUpdates=false\n",
        )?;
        let blank = "Kuma Preview";
        fs::write(
            scenes.join("Kuma.json"),
            serde_json::to_vec(&json!({
                "name":"Kuma","current_scene":blank,"current_program_scene":blank,
                "scene_order":[{"name":blank}],
                "sources":[{"name":blank,"id":"scene","versioned_id":"scene","settings":{"items":[]}}]
            }))?,
        )?;

        let width = (display.rect.right - display.rect.left) as u32;
        let height = (display.rect.bottom - display.rect.top) as u32;
        let scale = (settings.resolution as f64 / height as f64)
            .min(4096.0 / width as f64)
            .min(1.0);
        let even = |size: u32| ((size as f64 * scale).round() as u32 & !1).max(2);
        let (out_w, out_h) = (even(width), even(height));
        let stage = scratch.to_string_lossy().replace('\\', "/");
        let format = "[General]\nName=Kuma";
        let video = format!("[Video]\nBaseCX={out_w}\nBaseCY={out_h}\nOutputCX={out_w}\nOutputCY={out_h}\nFPSType=0\nFPSCommon={}\n", settings.fps.clamp(15, 120));
        let output =
            "[Output]\nFilenameFormatting=Kuma-%CCYY-%MM-%DD-%hh-%mm-%ss\nOverwriteIfExists=false";
        let seconds = settings.replay_seconds.clamp(5, 600);
        let mb = settings.replay_mb.clamp(32, 8192);
        let profile = if settings.bitrate > 0 && encoder == "x264" {
            fs::write(
                profiles.join("recordEncoder.json"),
                serde_json::to_vec(
                    &json!({"rate_control":"CBR","bitrate":settings.bitrate.clamp(500,100000),"keyint_sec":2,"preset":"veryfast"}),
                )?,
            )?;
            format!("{format}\n{output}\nMode=Advanced\n{video}\n[AdvOut]\nRecType=Standard\nRecFilePath={stage}\nRecFormat2=fragmented_mp4\nRecEncoder=obs_x264\nRecAudioEncoder=ffmpeg_aac\nRecTracks=1\nRecRB=true\nRecRBTime={seconds}\nRecRBSize={mb}\n")
        } else {
            let quality = if settings.quality == "HQ" {
                "HQ"
            } else {
                "Small"
            };
            format!("{format}\n{output}\nMode=Simple\n{video}\n[SimpleOutput]\nFilePath={stage}\nRecFormat2=fragmented_mp4\nRecQuality={quality}\nRecEncoder={encoder}\nRecAudioEncoder=aac\nRecTracks=1\nRecRB=true\nRecRBTime={seconds}\nRecRBSize={mb}\n")
        };
        fs::write(profiles.join("basic.ini"), profile)?;

        self.child = Some(
            Command::new(exe)
                .current_dir(&bin)
                .args([
                    "--portable",
                    "--multi",
                    "--minimize-to-tray",
                    "--disable-updater",
                    "--only-bundled-plugins",
                    "--disable-missing-files-check",
                    "--profile",
                    "Kuma",
                    "--collection",
                    "Kuma",
                ])
                .spawn()
                .context("Start bundled OBS")?,
        );
        let deadline = Instant::now() + Duration::from_secs(22);
        self.socket = Some(loop {
            if let Some(status) = self.child.as_mut().unwrap().try_wait()? {
                bail!("OBS stopped during startup ({status}).");
            }
            if let Ok(socket) = Socket::connect(port, &password) {
                break socket;
            }
            if Instant::now() >= deadline {
                bail!("OBS did not start its local capture connection. Check the OBS log in obs/config/obs-studio/logs.");
            }
            thread::sleep(Duration::from_millis(200));
        });

        let socket = self.socket()?;
        socket.call("CreateScene", json!({"sceneName":SCENE}))?;
        let item = socket.call("CreateInput", json!({"sceneName":SCENE,"inputName":INPUT,"inputKind":"monitor_capture","inputSettings":{"monitor_id":display.id,"capture_cursor":settings.cursor,"force_sdr":true}}))?;
        let monitors = socket.call(
            "GetInputPropertiesListPropertyItems",
            json!({"inputName":INPUT,"propertyName":"monitor_id"}),
        )?;
        let available = monitors["propertyItems"].as_array().is_some_and(|items| {
            items.iter().any(|m| {
                m["itemValue"]
                    .as_str()
                    .is_some_and(|id| id.eq_ignore_ascii_case(&display.id))
            })
        });
        if !available {
            bail!("OBS could not match the selected monitor.");
        }
        socket.call("SetSceneItemTransform", json!({"sceneName":SCENE,"sceneItemId":item["sceneItemId"],"sceneItemTransform":{"positionX":0.0,"positionY":0.0,"alignment":5,"boundsType":"OBS_BOUNDS_SCALE_INNER","boundsWidth":out_w,"boundsHeight":out_h,"boundsAlignment":0}}))?;
        let mut devices = [Vec::new(), Vec::new()];
        for (index, (name, kind, device, enabled)) in [
            (
                DESKTOP,
                "wasapi_output_capture",
                &settings.desktop_device,
                settings.desktop_audio,
            ),
            (
                MIC,
                "wasapi_input_capture",
                &settings.mic_device,
                settings.microphone,
            ),
        ]
        .into_iter()
        .enumerate()
        {
            let selected = if enabled { device.as_str() } else { "" };
            socket.call("CreateInput", json!({"sceneName":SCENE,"inputName":name,"inputKind":kind,"inputSettings":{"device_id":selected},"sceneItemEnabled":enabled}))?;
            socket.call(
                "SetInputMute",
                json!({"inputName":name,"inputMuted":!enabled}),
            )?;
            let list = socket.call(
                "GetInputPropertiesListPropertyItems",
                json!({"inputName":name,"propertyName":"device_id"}),
            )?;
            if let Some(items) = list["propertyItems"].as_array() {
                devices[index] = items
                    .iter()
                    .filter_map(|item| {
                        Some((
                            item["itemName"].as_str()?.to_owned(),
                            item["itemValue"].as_str()?.to_owned(),
                        ))
                    })
                    .collect();
            }
        }
        for channel in 1..=2 {
            let global = socket.call(
                "GetInputSettings",
                json!({"inputName": if channel == 1 { "Desktop Audio" } else { "Mic/Aux" }}),
            );
            if global.is_ok() {
                let _ = socket.call("SetInputMute", json!({"inputName": if channel == 1 { "Desktop Audio" } else { "Mic/Aux" },"inputMuted":true}));
            }
        }
        socket.call("SetCurrentProgramScene", json!({"sceneName":SCENE}))?;
        self.devices = devices;
        self.display = Some(display);
        self.dimensions = format!("{out_w}x{out_h}");
        Ok(())
    }

    pub fn toggle_replay(&mut self, settings: &Settings) -> Result<()> {
        if self.replay.is_some() {
            self.socket()?.call_simple("StopReplayBuffer")?;
            self.replay = None;
            if self.recording.is_none() {
                self.stop();
            }
            return Ok(());
        }
        self.start_output(settings, "StartReplayBuffer")?;
        self.replay = Some(Instant::now());
        Ok(())
    }

    pub fn toggle_recording(&mut self, settings: &Settings) -> Result<Option<Media>> {
        if let Some(started) = self.recording {
            let socket = self.socket()?;
            socket
                .events
                .retain(|event| event["eventType"] != "RecordStateChanged");
            let reply = socket.call_simple("StopRecord")?;
            self.recording = None;
            let path = if reply["outputPath"].is_string() {
                reply["outputPath"].clone()
            } else {
                self.socket()?
                    .event_match("RecordStateChanged", 30, |event| {
                        event["outputPath"].is_string()
                    })?["outputPath"]
                    .clone()
            };
            let media = self.finish(
                settings,
                Kind::Recording,
                &path,
                started.elapsed().as_secs(),
            );
            if self.replay.is_none() {
                self.stop();
            }
            return media.map(Some);
        }
        self.start_output(settings, "StartRecord")?;
        self.recording = Some(Instant::now());
        Ok(None)
    }

    fn start_output(&mut self, settings: &Settings, request: &str) -> Result<()> {
        if self.socket.is_some() && (self.recording.is_some() || self.replay.is_some()) {
            return self.socket()?.call_simple(request).map(|_| ());
        }
        let encoder = if settings.bitrate > 0 {
            "x264"
        } else {
            settings.encoder.as_str()
        };
        let result = self
            .start_engine(settings, encoder)
            .and_then(|()| self.socket()?.call_simple(request).map(|_| ()));
        if let Err(error) = result {
            self.stop();
            if encoder == "x264" {
                return Err(error);
            }
            let fallback = self
                .start_engine(settings, "x264")
                .and_then(|()| self.socket()?.call_simple(request).map(|_| ()));
            if let Err(error) = fallback {
                self.stop();
                return Err(error).context("Software encoder fallback failed");
            }
            self.fallback = true;
        }
        Ok(())
    }

    pub fn inspect_devices(&mut self, settings: &Settings) -> Result<()> {
        if self.socket.is_none() {
            let result = self.start_engine(settings, "x264");
            if result.is_err() {
                self.stop();
            }
            result?;
            self.stop();
        }
        Ok(())
    }

    pub fn save_replay(&mut self, settings: &Settings) -> Result<Media> {
        let started = self.replay.context("Arm instant replay first.")?;
        if started.elapsed().as_secs() < 2 {
            bail!("The replay buffer is still filling. Try again in a moment.");
        }
        let socket = self.socket()?;
        socket
            .events
            .retain(|event| event["eventType"] != "ReplayBufferSaved");
        socket.call_simple("SaveReplayBuffer")?;
        let saved = socket.event("ReplayBufferSaved", 30)?;
        self.finish(
            settings,
            Kind::Clip,
            &saved["savedReplayPath"],
            started
                .elapsed()
                .as_secs()
                .min(settings.replay_seconds as u64),
        )
    }

    fn finish(
        &self,
        settings: &Settings,
        kind: Kind,
        path: &Value,
        estimated_seconds: u64,
    ) -> Result<Media> {
        let original = PathBuf::from(
            path.as_str()
                .context("OBS did not return a saved file path.")?,
        );
        let mut source = fs::File::open(&original)?;
        let now = Local::now();
        let duration = mp4_duration(&original).or(Some(estimated_seconds));
        let info = NameInfo {
            kind,
            timestamp: now,
            monitor: self
                .display
                .as_ref()
                .map_or("Display", |display| display.label.as_str()),
            dimensions: &self.dimensions,
            duration,
            ext: "mp4",
        };
        let path = model::save_unique(&settings.folders[kind.id()], &info, |file| {
            std::io::copy(&mut source, file)?;
            Ok(())
        })
        .with_context(|| format!("Recovery copy retained at {}", original.display()))?;
        drop(source);
        let _ = fs::remove_file(original);
        if settings.copy_path {
            let _ = arboard::Clipboard::new()
                .and_then(|mut clipboard| clipboard.set_text(path.to_string_lossy().into_owned()));
        }
        Ok(Media {
            kind,
            path,
            timestamp: now,
            dimensions: self.dimensions.clone(),
            duration,
            upload: String::new(),
            link: String::new(),
            upload_profile: String::new(),
        })
    }

    pub fn tick(&mut self, settings: &Settings) -> Result<Option<Media>> {
        if let Some(child) = &mut self.child {
            if child.try_wait()?.is_some() {
                self.stop();
                bail!("The OBS capture runtime stopped.");
            }
        }
        if let Some(display) = &self.display {
            if !capture::displays().iter().any(|item| item.id == display.id) {
                let media = if self.recording.is_some() {
                    self.toggle_recording(settings)?
                } else {
                    None
                };
                self.stop();
                return if media.is_some() {
                    Ok(media)
                } else {
                    Err(anyhow::anyhow!(
                        "Capture paused because the selected monitor disconnected."
                    ))
                };
            }
        }
        let mut meters = self.meters;
        if let Some(socket) = &mut self.socket {
            socket.ws.get_mut().set_nonblocking(true)?;
            let update = (|| -> Result<()> {
                for _ in 0..40 {
                    let Some(event) = socket.poll()? else {
                        break;
                    };
                    if event["op"] == 5 {
                        socket.remember(event["d"].clone());
                    }
                }
                while let Some(event) = socket.events.pop_front() {
                    if event["eventType"] == "InputVolumeMeters" {
                        if let Some(inputs) = event["eventData"]["inputs"].as_array() {
                            for input in inputs {
                                let index = match input["inputName"].as_str() {
                                    Some(DESKTOP) => 0,
                                    Some(MIC) => 1,
                                    _ => continue,
                                };
                                let level = input["inputLevelsMul"]
                                    .as_array()
                                    .into_iter()
                                    .flatten()
                                    .filter_map(|channel| channel[1].as_f64())
                                    .fold(0.0, f64::max);
                                meters[index] = level.sqrt().clamp(0.0, 1.0) as f32;
                            }
                        }
                    }
                }
                Ok(())
            })();
            socket.ws.get_mut().set_nonblocking(false)?;
            update?;
        }
        self.meters = meters;
        Ok(None)
    }

    pub fn stop(&mut self) {
        self.socket = None;
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        self.display = None;
        self.replay = None;
        self.recording = None;
        self.meters = [0.0; 2];
        self.fallback = false;
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.stop();
    }
}

struct Atom {
    kind: [u8; 4],
    body: u64,
    end: u64,
}

fn atoms(file: &mut fs::File, mut position: u64, end: u64) -> Option<Vec<Atom>> {
    let mut atoms = Vec::new();
    while position + 8 <= end {
        file.seek(SeekFrom::Start(position)).ok()?;
        let mut header = [0u8; 8];
        file.read_exact(&mut header).ok()?;
        let short = u32::from_be_bytes(header[..4].try_into().ok()?);
        let (size, prefix) = match short {
            0 => (end - position, 8),
            1 => {
                let mut size = [0; 8];
                file.read_exact(&mut size).ok()?;
                (u64::from_be_bytes(size), 16)
            }
            _ => (short as u64, 8),
        };
        let next = position.checked_add(size)?;
        if size < prefix || next > end {
            return None;
        }
        atoms.push(Atom {
            kind: header[4..].try_into().ok()?,
            body: position + prefix,
            end: next,
        });
        position = next;
    }
    Some(atoms)
}

fn atom_bytes(file: &mut fs::File, atom: &Atom, limit: usize) -> Option<Vec<u8>> {
    let mut data = vec![0; ((atom.end - atom.body) as usize).min(limit)];
    file.seek(SeekFrom::Start(atom.body)).ok()?;
    file.read_exact(&mut data).ok()?;
    Some(data)
}

fn read_u32(bytes: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_be_bytes(
        bytes.get(offset..offset + 4)?.try_into().ok()?,
    ))
}

fn read_u64(bytes: &[u8], offset: usize) -> Option<u64> {
    Some(u64::from_be_bytes(
        bytes.get(offset..offset + 8)?.try_into().ok()?,
    ))
}

fn mp4_duration(path: &Path) -> Option<u64> {
    let mut file = fs::File::open(path).ok()?;
    let end = file.metadata().ok()?.len();
    let top = atoms(&mut file, 0, end)?;
    let moov = top.iter().find(|atom| atom.kind == *b"moov")?;
    let movie = atoms(&mut file, moov.body, moov.end)?;
    let mut scales = HashMap::new();
    let mut defaults = HashMap::new();
    let mut longest = 0.0_f64;
    for atom in movie {
        if atom.kind == *b"mvhd" {
            let data = atom_bytes(&mut file, &atom, 36)?;
            let v1 = *data.first()? == 1;
            let scale = read_u32(&data, if v1 { 20 } else { 12 })?;
            let ticks = if v1 {
                read_u64(&data, 24)?
            } else {
                read_u32(&data, 16)? as u64
            };
            if scale != 0 && ticks != u32::MAX as u64 && ticks != u64::MAX {
                longest = ticks as f64 / scale as f64;
            }
        } else if atom.kind == *b"trak" {
            let children = atoms(&mut file, atom.body, atom.end)?;
            let tkhd = children.iter().find(|node| node.kind == *b"tkhd")?;
            let mdia = children.iter().find(|node| node.kind == *b"mdia")?;
            let mdhd = atoms(&mut file, mdia.body, mdia.end)?
                .into_iter()
                .find(|node| node.kind == *b"mdhd")?;
            let track = atom_bytes(&mut file, tkhd, 24)?;
            let timing = atom_bytes(&mut file, &mdhd, 24)?;
            scales.insert(
                read_u32(&track, if *track.first()? == 1 { 20 } else { 12 })?,
                read_u32(&timing, if *timing.first()? == 1 { 20 } else { 12 })?,
            );
        } else if atom.kind == *b"mvex" {
            for child in atoms(&mut file, atom.body, atom.end)?
                .into_iter()
                .filter(|node| node.kind == *b"trex")
            {
                let data = atom_bytes(&mut file, &child, 16)?;
                defaults.insert(read_u32(&data, 4)?, read_u32(&data, 12)?);
            }
        }
    }
    for fragment in top.iter().filter(|node| node.kind == *b"moof") {
        for traf in atoms(&mut file, fragment.body, fragment.end)?
            .into_iter()
            .filter(|node| node.kind == *b"traf")
        {
            let children = atoms(&mut file, traf.body, traf.end)?;
            let Some(tfhd) = children.iter().find(|node| node.kind == *b"tfhd") else {
                continue;
            };
            let Some(tfdt) = children.iter().find(|node| node.kind == *b"tfdt") else {
                continue;
            };
            let header = atom_bytes(&mut file, tfhd, 32)?;
            let flags = read_u32(&header, 0)?;
            let track = read_u32(&header, 4)?;
            let Some(&scale) = scales.get(&track).filter(|scale| **scale != 0) else {
                continue;
            };
            let mut offset = 8;
            if flags & 1 != 0 {
                offset += 8;
            }
            if flags & 2 != 0 {
                offset += 4;
            }
            let default_duration = if flags & 8 != 0 {
                read_u32(&header, offset)?
            } else {
                defaults.get(&track).copied().unwrap_or_default()
            };
            let start = atom_bytes(&mut file, tfdt, 12)?;
            let mut ticks = if *start.first()? == 1 {
                read_u64(&start, 4)?
            } else {
                read_u32(&start, 4)? as u64
            };
            for run in children.iter().filter(|node| node.kind == *b"trun") {
                let data = atom_bytes(&mut file, run, 2 * 1024 * 1024)?;
                let flags = read_u32(&data, 0)?;
                let count = read_u32(&data, 4)? as usize;
                if count > 200_000 {
                    return None;
                }
                let mut sample =
                    8 + if flags & 1 != 0 { 4 } else { 0 } + if flags & 4 != 0 { 4 } else { 0 };
                let stride = (flags & 0xf00).count_ones() as usize * 4;
                if flags & 0x100 != 0 {
                    for _ in 0..count {
                        ticks += read_u32(&data, sample)? as u64;
                        sample += stride;
                    }
                } else {
                    ticks += count as u64 * default_duration as u64;
                }
            }
            longest = longest.max(ticks as f64 / scale as f64);
        }
    }
    (longest > 0.0).then(|| longest.round().max(1.0) as u64)
}
