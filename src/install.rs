use std::{
    env, fs,
    path::PathBuf,
    process::{Command, Stdio},
};

const CLM_COMPOSE: &str = include_str!("../deploy/clm/compose.yaml");
const CLM_PORT: u16 = 8700;

/// Our own clm container already holding the port is fine: `up -d` leaves it running.
pub fn clm_running() -> bool {
    running("jeff-clm-clm")
}

/// Qwen3-8B safetensors, summed from the Hugging Face file list.
const ENCODER_WEIGHT_BYTES: u64 = 16_381_516_776;
/// CLM_v0.1-8B.pt, the reference head clm-serve downloads on first start.
const CLM_HEAD_BYTES: u64 = 75_557_149;

fn running(name: &str) -> bool {
    Command::new("docker")
        .args([
            "ps",
            "-q",
            "--filter",
            &format!("name={name}"),
            "--filter",
            "status=running",
        ])
        .output()
        .is_ok_and(|o| !o.stdout.is_empty())
}

fn bytes_in(container: &str, path: &str) -> u64 {
    let out = Command::new("docker")
        .args(["exec", container, "du", "-sb", path])
        .output();
    out.ok()
        .and_then(|o| {
            String::from_utf8_lossy(&o.stdout)
                .split_whitespace()
                .next()?
                .parse()
                .ok()
        })
        .unwrap_or(0)
}

/// Install steps in order, each `{id, label, weight, done, progress?}`. `weight` is the step's share of the whole
/// install in rough gigabytes, so the overall progress tracks the downloads. `loaded` is whether clm-serve reaches its encoder;
/// `warm` is whether a first request has gone through, which takes about 30 s after the encoder loads.
pub fn clm_steps(loaded: bool, warm: bool) -> Vec<serde_json::Value> {
    use serde_json::json;
    let docker = docker_ready().unwrap_or(false);
    let image = docker
        && Command::new("docker")
            .args(["image", "inspect", "vllm/vllm-openai@sha256:6bf34e50e2387dc46dc87a9d6a945fdd616a022bccfddd949052f54063ebcb8c"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
    let encoder_up = docker && running("jeff-clm-encoder");
    let clm_up = docker && running("jeff-clm-clm");
    let weights = if encoder_up {
        bytes_in(
            "jeff-clm-encoder-1",
            "/root/.cache/huggingface/hub/models--Qwen--Qwen3-8B",
        )
    } else {
        0
    };
    let head = if clm_up {
        bytes_in("jeff-clm-clm-1", "/data")
    } else {
        0
    };
    let ratio = |n: u64, total: u64| (n as f64 / total as f64).min(1.0);
    vec![
        json!({ "id": "docker", "label": "Start Docker", "weight": 0.5, "done": docker }),
        json!({ "id": "image", "label": "Download the vLLM image (about 10 GB)", "weight": 10.0, "done": image }),
        json!({ "id": "containers", "label": "Start the containers", "weight": 0.2, "done": encoder_up && clm_up }),
        json!({ "id": "weights", "label": "Download Qwen3-8B weights (16.4 GB)", "weight": 16.4,
                "done": loaded || weights >= ENCODER_WEIGHT_BYTES, "progress": ratio(weights, ENCODER_WEIGHT_BYTES) }),
        json!({ "id": "head", "label": "Download the CLM head (76 MB)", "weight": 0.1,
                "done": head >= CLM_HEAD_BYTES, "progress": ratio(head, CLM_HEAD_BYTES) }),
        json!({ "id": "load", "label": "Load the encoder on the GPU", "weight": 1.0, "done": loaded }),
        json!({ "id": "warm", "label": "Warm up the model", "weight": 0.5, "done": loaded && warm }),
    ]
}

/// Where jeff keeps its config and compose files: `JEFF_HOME`, else `~/.jeff`.
pub fn home() -> PathBuf {
    if let Ok(dir) = env::var("JEFF_HOME") {
        return dir.into();
    }
    let user = env::var("USERPROFILE")
        .or_else(|_| env::var("HOME"))
        .unwrap_or_else(|_| ".".into());
    PathBuf::from(user).join(".jeff")
}

fn clm_compose_file() -> Result<PathBuf, String> {
    let dir = home().join("clm");
    fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let file = dir.join("compose.yaml");
    fs::write(&file, CLM_COMPOSE).map_err(|e| format!("{}: {e}", file.display()))?;
    Ok(file)
}

#[derive(Clone, Copy)]
pub enum Action {
    Install,
    Remove,
}

impl Action {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "install" => Some(Self::Install),
            "remove" => Some(Self::Remove),
            _ => None,
        }
    }

    fn compose_args(self) -> &'static [&'static str] {
        match self {
            // Without --force-recreate, Reinstall on running containers would do nothing.
            Self::Install => &["up", "-d", "--force-recreate"],
            // Volumes stay, so a reinstall does not download the 16 GB of weights again.
            Self::Remove => &["down"],
        }
    }
}

fn docker_ready() -> Result<bool, String> {
    match Command::new("docker")
        .arg("info")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
    {
        Ok(s) => Ok(s.success()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(
            "Docker not found. Install Docker Desktop: https://docs.docker.com/desktop/".to_owned(),
        ),
        Err(e) => Err(format!("docker: {e}")),
    }
}

fn start_docker_desktop() -> Result<(), String> {
    let started = if cfg!(windows) {
        let program_files = env::var("ProgramFiles").unwrap_or_else(|_| r"C:\Program Files".into());
        Command::new(PathBuf::from(program_files).join(r"Docker\Docker\Docker Desktop.exe"))
            .spawn()
            .is_ok()
    } else if cfg!(target_os = "macos") {
        Command::new("open")
            .args(["-a", "Docker"])
            .status()
            .is_ok_and(|s| s.success())
    } else {
        false
    };
    if started {
        Ok(())
    } else {
        Err("Docker is not running. Start the Docker daemon and try again.".to_owned())
    }
}

/// Starts Docker Desktop when the daemon is down and waits for it.
fn ensure_docker(progress: &dyn Fn(&str)) -> Result<(), String> {
    if docker_ready()? {
        return Ok(());
    }
    progress("Starting Docker Desktop…");
    start_docker_desktop()?;
    // Docker Desktop usually needs 20–60 s to bring up its WSL 2 VM.
    for _ in 0..90 {
        std::thread::sleep(std::time::Duration::from_secs(2));
        if docker_ready()? {
            return Ok(());
        }
    }
    Err("Docker Desktop did not start within 3 minutes. Open it manually and try again.".to_owned())
}

/// Runs `docker compose` for CLM. `inherit` streams progress to this terminal; otherwise output is captured.
pub fn clm(action: Action, inherit: bool, progress: &dyn Fn(&str)) -> Result<String, String> {
    ensure_docker(progress)?;
    if matches!(action, Action::Install) && !port_free(CLM_PORT) && !clm_running() {
        return Err(format!(
            "Port {CLM_PORT} is taken by another program, perhaps a clm-serve started by hand. Stop it and press Install again."
        ));
    }
    progress(match action {
        Action::Install => "Pulling images and starting CLM… the first pull is about 10 GB.",
        Action::Remove => "Removing CLM containers…",
    });
    let file = clm_compose_file()?;
    let mut cmd = Command::new("docker");
    cmd.arg("compose")
        .arg("-f")
        .arg(&file)
        .args(action.compose_args());
    if inherit {
        cmd.stdout(Stdio::inherit()).stderr(Stdio::inherit());
    }
    let out = cmd.output().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            "Docker not found. Install Docker Desktop: https://docs.docker.com/desktop/".to_owned()
        } else {
            format!("docker: {e}")
        }
    })?;
    let text = String::from_utf8_lossy(&out.stderr).trim().to_owned();
    if out.status.success() {
        Ok(text)
    } else {
        // Compose prints its whole progress log; the cause is the last line.
        Err(text
            .lines()
            .last()
            .unwrap_or("docker compose failed")
            .trim()
            .to_owned())
    }
}

fn port_free(port: u16) -> bool {
    std::net::TcpListener::bind(("127.0.0.1", port)).is_ok()
}
