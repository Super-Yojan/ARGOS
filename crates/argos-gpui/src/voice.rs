//! Spoken tasks for the agent engine.
//!
//! The operator presses Talk in the Agent panel (or Cmd/Ctrl+Shift+V), says
//! what the fleet should do, and presses again. [`Recording`] captures the
//! default microphone, [`Transcriber`] turns the clip into text off the UI
//! thread, and the window hands the text to the picked engine (Claude, Codex,
//! OpenCode) as the operator's task. The engine only plans: its moves come
//! back as recommendation cards the operator accepts or rejects.
//!
//! | Transcriber | Picked when | Runs |
//! | --- | --- | --- |
//! | Command | `ARGOS_STT_CMD` is set | That shell command; `{wav}` is the clip, stdout is the text |
//! | Whisper | `whisper-cli` (or `whisper-cpp`) is on PATH and a `ggml-*.bin` model is found | Locally, offline |
//! | OpenAI | `OPENAI_API_KEY` is set | `curl` to `/v1/audio/transcriptions` (`whisper-1`) |
//!
//! The Whisper model is `ARGOS_WHISPER_MODEL`, else the best `ggml-*.bin` in
//! `~/.cache/whisper`.
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SizedSample};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// Whisper models expect 16 kHz mono.
pub const SAMPLE_RATE: u32 = 16_000;
/// A clip stops here even if the operator forgets to press Talk again.
pub const MAX_RECORD_S: f64 = 60.;
/// Shorter clips are a mis-click, not a task.
pub const MIN_RECORD_S: f64 = 0.4;
/// Words the transcriber should expect, so "L3" and "Terra" come out right.
pub const VOCABULARY: &str = "ARGOS, Terra, rover, rovers, L0, L1, L2, L3, L4, L5, waypoint, \
explore, bucket, buckets, ball, balls, dock, dump, home, frontier, operator.";
/// The model Talk downloads when whisper.cpp is installed without one.
pub const DEFAULT_MODEL: &str = "ggml-base.en.bin";
/// Where [`DEFAULT_MODEL`] comes from (the whisper.cpp project's own copy).
pub const DEFAULT_MODEL_URL: &str =
    "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.en.bin";
/// A finished download is at least this big (the real file is ~148 MB).
const MIN_MODEL_BYTES: u64 = 50_000_000;
/// Program folders an app opened from Finder does not get on PATH.
const EXTRA_BIN_DIRS: &[&str] = &["/opt/homebrew/bin", "/usr/local/bin", "/opt/local/bin"];
/// Model files tried in `~/.cache/whisper`, best first.
const MODEL_PREFERENCE: &[&str] = &[
    "ggml-base.en.bin",
    "ggml-small.en.bin",
    "ggml-base.bin",
    "ggml-small.bin",
    "ggml-tiny.en.bin",
    "ggml-medium.en.bin",
    "ggml-large-v3-turbo.bin",
];

/// The microphone while the operator talks. Dropping it stops the capture.
pub struct Recording {
    _stream: cpal::Stream,
    samples: Arc<Mutex<Vec<f32>>>,
    rate: u32,
    channels: u16,
    started: Instant,
    error: Arc<Mutex<Option<String>>>,
}

impl Recording {
    /// Open the default input device and start capturing.
    pub fn start() -> Result<Self, String> {
        let host = cpal::default_host();
        let device = host
            .default_input_device()
            .ok_or("No microphone found. Check the system sound input settings.")?;
        let config = device
            .default_input_config()
            .map_err(|error| format!("Microphone not usable: {error}"))?;
        let rate = config.sample_rate().0;
        let channels = config.channels();
        let cap = (MAX_RECORD_S * f64::from(rate) * f64::from(channels)) as usize;
        let samples = Arc::new(Mutex::new(Vec::with_capacity(cap.min(1 << 22))));
        let error = Arc::new(Mutex::new(None));
        let stream_config: cpal::StreamConfig = config.clone().into();
        let stream = match config.sample_format() {
            cpal::SampleFormat::F32 => build::<f32>(&device, &stream_config, &samples, &error, cap),
            cpal::SampleFormat::I16 => build::<i16>(&device, &stream_config, &samples, &error, cap),
            cpal::SampleFormat::U16 => build::<u16>(&device, &stream_config, &samples, &error, cap),
            cpal::SampleFormat::I32 => build::<i32>(&device, &stream_config, &samples, &error, cap),
            other => return Err(format!("Microphone sample format {other} is not supported")),
        }
        .map_err(|error| format!("Could not open the microphone: {error}"))?;
        stream
            .play()
            .map_err(|error| format!("Could not start the microphone: {error}"))?;
        Ok(Self {
            _stream: stream,
            samples,
            rate,
            channels,
            started: Instant::now(),
            error,
        })
    }

    pub fn seconds(&self) -> f64 {
        self.started.elapsed().as_secs_f64()
    }

    /// An error the audio driver reported while recording, if any.
    pub fn error(&self) -> Option<String> {
        self.error.lock().ok().and_then(|error| error.clone())
    }

    /// Stop and return the clip as 16 kHz mono samples.
    pub fn finish(self) -> Vec<i16> {
        let raw = self
            .samples
            .lock()
            .map(|samples| samples.clone())
            .unwrap_or_default();
        to_mono_16k(&raw, self.channels, self.rate)
    }
}

fn build<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    samples: &Arc<Mutex<Vec<f32>>>,
    error: &Arc<Mutex<Option<String>>>,
    cap: usize,
) -> Result<cpal::Stream, cpal::BuildStreamError>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    let samples = samples.clone();
    let error = error.clone();
    device.build_input_stream(
        config,
        move |data: &[T], _: &cpal::InputCallbackInfo| {
            if let Ok(mut samples) = samples.lock() {
                let room = cap.saturating_sub(samples.len());
                samples.extend(
                    data.iter()
                        .take(room)
                        .map(|sample| sample.to_sample::<f32>()),
                );
            }
        },
        move |stream_error| {
            if let Ok(mut error) = error.lock() {
                *error = Some(stream_error.to_string());
            }
        },
        None,
    )
}

/// Average the channels and resample to [`SAMPLE_RATE`] (linear), as i16.
pub fn to_mono_16k(interleaved: &[f32], channels: u16, rate: u32) -> Vec<i16> {
    let channels = usize::from(channels.max(1));
    let mono: Vec<f32> = interleaved
        .chunks(channels)
        .map(|frame| frame.iter().sum::<f32>() / frame.len() as f32)
        .collect();
    if mono.is_empty() || rate == 0 {
        return Vec::new();
    }
    let step = f64::from(rate) / f64::from(SAMPLE_RATE);
    let count = (mono.len() as f64 / step).floor() as usize;
    (0..count)
        .map(|index| {
            let at = index as f64 * step;
            let low = at.floor() as usize;
            let high = (low + 1).min(mono.len() - 1);
            let frac = (at - low as f64) as f32;
            let value = mono[low] * (1. - frac) + mono[high] * frac;
            (value.clamp(-1., 1.) * f32::from(i16::MAX)) as i16
        })
        .collect()
}

/// A 16-bit PCM mono WAV file.
pub fn wav_bytes(samples: &[i16], rate: u32) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for sample in samples {
        out.extend_from_slice(&sample.to_le_bytes());
    }
    out
}

/// Seconds of audio in a 16 kHz clip.
pub fn clip_seconds(samples: &[i16]) -> f64 {
    samples.len() as f64 / f64::from(SAMPLE_RATE)
}

/// How speech becomes text on this computer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Transcriber {
    /// `ARGOS_STT_CMD`, run through the shell with `{wav}` replaced.
    Command(String),
    /// whisper.cpp's CLI with a local model.
    Whisper { program: PathBuf, model: PathBuf },
    /// OpenAI's hosted transcription, through `curl`.
    OpenAi { key: String, base: String },
}

impl Transcriber {
    /// Pick from the environment, in the order of the table in the module docs.
    pub fn pick(
        env: impl Fn(&str) -> Option<String>,
        find: impl Fn(&str) -> Option<PathBuf>,
        home: Option<PathBuf>,
    ) -> Result<Self, String> {
        let set = |name: &str| env(name).filter(|value| !value.trim().is_empty());
        if let Some(command) = set("ARGOS_STT_CMD") {
            return Ok(Self::Command(command));
        }
        let program = find("whisper-cli").or_else(|| find("whisper-cpp"));
        if let Some(program) = program {
            let model = set("ARGOS_WHISPER_MODEL").map(PathBuf::from).or_else(|| {
                let dir = home?.join(".cache").join("whisper");
                MODEL_PREFERENCE
                    .iter()
                    .map(|name| dir.join(name))
                    .find(|path| path.is_file())
            });
            if let Some(model) = model {
                return Ok(Self::Whisper { program, model });
            }
        }
        if let Some(key) = set("OPENAI_API_KEY") {
            let base = set("OPENAI_BASE_URL")
                .unwrap_or_else(|| "https://api.openai.com/v1".into())
                .trim_end_matches('/')
                .to_string();
            return Ok(Self::OpenAi { key, base });
        }
        Err(
            "No speech-to-text found. Install whisper.cpp (`brew install whisper-cpp`) and put \
             ggml-base.en.bin in ~/.cache/whisper, set OPENAI_API_KEY, or set ARGOS_STT_CMD. \
             You can also type the task."
                .into(),
        )
    }

    /// Pick from this process's environment and PATH.
    pub fn from_env() -> Result<Self, String> {
        Self::pick(|name| std::env::var(name).ok(), find_on_path, home_dir())
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Command(_) => "ARGOS_STT_CMD",
            Self::Whisper { .. } => "Whisper (local)",
            Self::OpenAi { .. } => "OpenAI",
        }
    }

    /// Turn a WAV file into text. Blocking: call it off the UI thread.
    pub fn transcribe(&self, wav: &Path) -> Result<String, String> {
        let output = match self {
            Self::Command(template) => {
                let quoted = format!("\"{}\"", wav.display());
                let line = if template.contains("{wav}") {
                    template.replace("{wav}", &quoted)
                } else {
                    format!("{template} {quoted}")
                };
                shell(&line).output()
            }
            Self::Whisper { program, model } => Command::new(program)
                .arg("-m")
                .arg(model)
                .arg("-f")
                .arg(wav)
                .args(["-l", "en", "-nt", "-np", "--prompt", VOCABULARY])
                .stdin(Stdio::null())
                .output(),
            Self::OpenAi { key, base } => {
                // The key goes in on stdin, so it never shows in a process list.
                let mut child = Command::new("curl")
                    .args(["-sS", "--max-time", "60", "-H", "@-"])
                    .arg("-F")
                    .arg(format!("file=@{}", wav.display()))
                    .args(["-F", "model=whisper-1", "-F", "response_format=text"])
                    .arg("-F")
                    .arg(format!("prompt={VOCABULARY}"))
                    .arg(format!("{base}/audio/transcriptions"))
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()
                    .map_err(|error| format!("Could not run curl: {error}"))?;
                if let Some(mut stdin) = child.stdin.take() {
                    let _ = writeln!(stdin, "Authorization: Bearer {key}");
                }
                child.wait_with_output()
            }
        }
        .map_err(|error| format!("{} could not start: {error}", self.label()))?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let last = stderr.lines().rev().find(|line| !line.trim().is_empty());
            return Err(format!(
                "{} failed: {}",
                self.label(),
                last.unwrap_or("no error text")
            ));
        }
        if matches!(self, Self::OpenAi { .. }) && stdout.trim_start().starts_with('{') {
            let message = serde_json::from_str::<serde_json::Value>(&stdout)
                .ok()
                .and_then(|value| value["error"]["message"].as_str().map(str::to_string))
                .unwrap_or_else(|| stdout.trim().to_string());
            return Err(format!("OpenAI: {message}"));
        }
        let text = clean_transcript(&stdout);
        if text.is_empty() {
            Err("Heard nothing. Speak closer to the microphone, or type the task.".into())
        } else {
            Ok(text)
        }
    }
}

/// One line of plain text: no timestamps, no `[BLANK_AUDIO]`-style tags.
pub fn clean_transcript(raw: &str) -> String {
    let mut words = Vec::new();
    for line in raw.lines() {
        let mut line = line.trim();
        // whisper.cpp without -nt: "[00:00:00.000 --> 00:00:02.000]   text"
        if line.starts_with('[')
            && let Some(end) = line.find(']')
            && line[..end].contains("-->")
        {
            line = &line[end + 1..];
        }
        let mut rest = line;
        let mut kept = String::new();
        while let Some(start) = rest.find(['[', '(']) {
            let close = if rest[start..].starts_with('[') {
                ']'
            } else {
                ')'
            };
            kept.push_str(&rest[..start]);
            match rest[start..].find(close) {
                Some(end) => {
                    let inner = &rest[start + 1..start + end];
                    let tag = inner
                        .chars()
                        .all(|c| c.is_ascii_uppercase() || c == '_' || c == ' ');
                    let noise = ["music", "silence", "inaudible", "blank audio"]
                        .contains(&inner.trim().to_ascii_lowercase().as_str());
                    if !(tag || noise) {
                        kept.push_str(&rest[start..start + end + 1]);
                    }
                    rest = &rest[start + end + 1..];
                }
                None => {
                    kept.push_str(&rest[start..]);
                    rest = "";
                }
            }
        }
        kept.push_str(rest);
        words.extend(kept.split_whitespace().map(str::to_string));
    }
    words.join(" ")
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

/// whisper.cpp is installed but has no model: the program and where
/// [`DEFAULT_MODEL`] should go. `None` when something else is set up or
/// whisper.cpp is missing.
pub fn missing_model() -> Option<(PathBuf, PathBuf)> {
    if std::env::var("ARGOS_STT_CMD").is_ok_and(|value| !value.trim().is_empty()) {
        return None;
    }
    let program = find_on_path("whisper-cli").or_else(|| find_on_path("whisper-cpp"))?;
    let dir = home_dir()?.join(".cache").join("whisper");
    if MODEL_PREFERENCE.iter().any(|name| dir.join(name).is_file()) {
        return None;
    }
    Some((program, dir.join(DEFAULT_MODEL)))
}

/// Download [`DEFAULT_MODEL`] to `dest` with curl, through a `.part` file so
/// a cut-off download never looks like a model. Blocking.
pub fn download_model(dest: &Path) -> Result<(), String> {
    let dir = dest.parent().ok_or("bad model path")?;
    std::fs::create_dir_all(dir)
        .map_err(|error| format!("Could not make {}: {error}", dir.display()))?;
    let part = dest.with_extension("bin.part");
    let output = Command::new("curl")
        .args(["-sSL", "--fail", "--retry", "2", "--max-time", "900", "-o"])
        .arg(&part)
        .arg(DEFAULT_MODEL_URL)
        .stdin(Stdio::null())
        .output()
        .map_err(|error| format!("Could not run curl: {error}"))?;
    let size = std::fs::metadata(&part).map(|meta| meta.len()).unwrap_or(0);
    if !output.status.success() || size < MIN_MODEL_BYTES {
        let _ = std::fs::remove_file(&part);
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "Could not download the Whisper model ({}). Download {DEFAULT_MODEL_URL} into {} by hand.",
            stderr.trim().lines().last().unwrap_or("too small"),
            dir.display()
        ));
    }
    std::fs::rename(&part, dest).map_err(|error| format!("Could not save the model: {error}"))
}

fn shell(line: &str) -> Command {
    if cfg!(windows) {
        let mut command = Command::new("cmd");
        command.args(["/C", line]);
        command
    } else {
        let mut command = Command::new("sh");
        command.args(["-c", line]);
        command
    }
}

/// PATH, then the Homebrew/MacPorts folders that an app opened from Finder
/// does not inherit.
fn find_on_path(program: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    let names: Vec<String> = if cfg!(windows) {
        vec![format!("{program}.exe"), program.to_string()]
    } else {
        vec![program.to_string()]
    };
    std::env::split_paths(&path)
        .chain(EXTRA_BIN_DIRS.iter().map(PathBuf::from))
        .flat_map(|dir| names.iter().map(move |name| dir.join(name)))
        .find(|candidate| candidate.is_file())
}

/// Write the clip into `dir` and transcribe it. The file stays next to the
/// session log so a study run can check what was heard.
pub fn transcribe_clip(
    transcriber: &Transcriber,
    samples: &[i16],
    dir: &Path,
    name: &str,
) -> Result<(String, PathBuf), String> {
    std::fs::create_dir_all(dir).map_err(|error| error.to_string())?;
    let path = dir.join(name);
    std::fs::write(&path, wav_bytes(samples, SAMPLE_RATE))
        .map_err(|error| format!("Could not save the clip: {error}"))?;
    transcriber.transcribe(&path).map(|text| (text, path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resamples_stereo_48k_to_mono_16k() {
        let frames = 4_800; // 0.1 s
        let stereo: Vec<f32> = (0..frames).flat_map(|_| [0.5, -0.5]).collect();
        let out = to_mono_16k(&stereo, 2, 48_000);
        assert_eq!(out.len(), 1_600);
        assert!(out.iter().all(|sample| *sample == 0));
        let loud = to_mono_16k(&vec![1.0; 4_410], 1, 44_100);
        assert_eq!(loud.len(), 1_600);
        assert!(loud.iter().all(|sample| *sample == i16::MAX));
    }

    #[test]
    fn wav_header_is_pcm_mono() {
        let wav = wav_bytes(&[0, 1, -1], SAMPLE_RATE);
        assert_eq!(wav.len(), 44 + 6);
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..16], b"WAVEfmt ");
        assert_eq!(u16::from_le_bytes([wav[22], wav[23]]), 1);
        assert_eq!(
            u32::from_le_bytes([wav[24], wav[25], wav[26], wav[27]]),
            SAMPLE_RATE
        );
        assert_eq!(u32::from_le_bytes([wav[40], wav[41], wav[42], wav[43]]), 6);
    }

    #[test]
    fn cleans_whisper_output() {
        assert_eq!(
            clean_transcript("[00:00:00.000 --> 00:00:03.000]   Send rover 2 to explore.\n"),
            "Send rover 2 to explore."
        );
        assert_eq!(clean_transcript(" [BLANK_AUDIO]\n"), "");
        assert_eq!(
            clean_transcript("(music) Rover one, go home [inaudible] now"),
            "Rover one, go home now"
        );
        assert_eq!(
            clean_transcript("Explore the area (left side) first"),
            "Explore the area (left side) first"
        );
    }

    #[test]
    fn picks_transcriber_in_order() {
        let none = |_: &str| None::<String>;
        let nowhere = |_: &str| None::<PathBuf>;
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            move |name: &str| {
                pairs
                    .iter()
                    .find(|(key, _)| *key == name)
                    .map(|(_, value)| value.to_string())
            }
        };
        assert!(Transcriber::pick(none, nowhere, None).is_err());
        assert_eq!(
            Transcriber::pick(
                env(&[("ARGOS_STT_CMD", "hear -i {wav}"), ("OPENAI_API_KEY", "k")]),
                nowhere,
                None
            ),
            Ok(Transcriber::Command("hear -i {wav}".into()))
        );
        let whisper =
            |name: &str| (name == "whisper-cli").then(|| PathBuf::from("/bin/whisper-cli"));
        assert_eq!(
            Transcriber::pick(
                env(&[
                    ("ARGOS_WHISPER_MODEL", "/m/ggml-base.en.bin"),
                    ("OPENAI_API_KEY", "k")
                ]),
                whisper,
                None
            ),
            Ok(Transcriber::Whisper {
                program: "/bin/whisper-cli".into(),
                model: "/m/ggml-base.en.bin".into()
            })
        );
        // whisper-cli without a model falls through to OpenAI.
        assert_eq!(
            Transcriber::pick(env(&[("OPENAI_API_KEY", "k")]), whisper, None),
            Ok(Transcriber::OpenAi {
                key: "k".into(),
                base: "https://api.openai.com/v1".into()
            })
        );
    }

    #[test]
    fn command_transcriber_reads_stdout() {
        if cfg!(windows) {
            return;
        }
        let dir = std::env::temp_dir().join(format!("argos-voice-{}", std::process::id()));
        let transcriber = Transcriber::Command("echo '  Rover 1, explore for 5 min ' #".into());
        let (text, path) = transcribe_clip(&transcriber, &[0; 1600], &dir, "clip.wav").unwrap();
        assert_eq!(text, "Rover 1, explore for 5 min");
        assert!(path.is_file());
        let _ = std::fs::remove_dir_all(dir);
    }
}
