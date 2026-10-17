use std::{
    env, fs,
    path::PathBuf,
    process::{Command, Stdio},
    sync::{Arc, Mutex},
};

use serde_json::{Value, json};

const CLM_IMAGE: &str =
    "vllm/vllm-openai@sha256:6bf34e50e2387dc46dc87a9d6a945fdd616a022bccfddd949052f54063ebcb8c";
const H2O_IMAGE: &str =
    "vllm/vllm-openai@sha256:8a69ffad015f138d7170c4ddc429e230a3bc1c1719f67e14324749df200a4b90";

/// Qwen3-8B safetensors, summed from the Hugging Face file list.
const ENCODER_WEIGHT_BYTES: u64 = 16_381_516_776;
/// CLM_v0.1-8B.pt, the reference head clm-serve downloads on first start.
const CLM_HEAD_BYTES: u64 = 75_557_149;
/// model.safetensors of H2O-Lightning-4B v1.2.3.
const H2O_WEIGHT_BYTES: u64 = 9_078_620_536;

/// A local provider jeff installs in Docker.
#[derive(Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Stack {
    Clm,
    H2oLightning,
}

impl Stack {
    pub const ALL: [Stack; 2] = [Stack::Clm, Stack::H2oLightning];

    /// The provider's `installer` value and the path segment of its install endpoints.
    pub fn id(self) -> &'static str {
        match self {
            Self::Clm => "clm",
            Self::H2oLightning => "h2o-lightning",
        }
    }

    pub fn parse(installer: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.id() == installer)
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Clm => "CLM",
            Self::H2oLightning => "H2O-Lightning",
        }
    }

    fn compose(self) -> &'static str {
        match self {
            Self::Clm => include_str!("../deploy/clm/compose.yaml"),
            Self::H2oLightning => include_str!("../deploy/h2o-lightning/compose.yaml"),
        }
    }

    pub fn port(self) -> u16 {
        match self {
            Self::Clm => 8700,
            Self::H2oLightning => 8741,
        }
    }

    pub fn first_start(self) -> &'static str {
        match self {
            Self::Clm => "the encoder downloads ~16 GB on first start",
            Self::H2oLightning => "vLLM downloads ~9 GB of weights on first start",
        }
    }

    /// Our own server container already holding the port is fine: `up -d` leaves it running.
    pub fn running(self) -> bool {
        running(match self {
            Self::Clm => "jeff-clm-clm",
            Self::H2oLightning => "jeff-h2o-lightning-shim",
        })
    }

    /// Install steps in order, each `{id, label, weight, done, progress?}`. `weight` is the step's share of the whole
    /// install in rough gigabytes, so the overall progress tracks the downloads. `loaded` is whether the server reaches
    /// its model; `warm` is whether a first request has gone through.
    pub fn steps(self, loaded: bool, warm: bool) -> Vec<serde_json::Value> {
        match self {
            Self::Clm => clm_steps(loaded, warm),
            Self::H2oLightning => h2o_steps(loaded, warm),
        }
    }
}

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

fn image_present(image: &str) -> bool {
    Command::new("docker")
        .args(["image", "inspect", image])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

fn bytes_in(container: &str, path: &str) -> u64 {
    let out = Command::new("docker")
        .args(["exec", container, "du", "-sbL", path])
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

fn ratio(n: u64, total: u64) -> f64 {
    (n as f64 / total as f64).min(1.0)
}

/// clm-serve's first request takes about 30 s after the encoder loads.
fn clm_steps(loaded: bool, warm: bool) -> Vec<serde_json::Value> {
    use serde_json::json;
    let docker = docker_ready().unwrap_or(false);
    let image = docker && image_present(CLM_IMAGE);
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

fn h2o_steps(loaded: bool, warm: bool) -> Vec<serde_json::Value> {
    use serde_json::json;
    let docker = docker_ready().unwrap_or(false);
    let image = docker && image_present(H2O_IMAGE);
    let vllm_up = docker && running("jeff-h2o-lightning-vllm");
    let shim_up = docker && running("jeff-h2o-lightning-shim");
    let weights = if vllm_up {
        bytes_in(
            "jeff-h2o-lightning-vllm-1",
            "/root/.cache/huggingface/hub/models--h2oai--h2o-lightning-4b",
        )
    } else {
        0
    };
    vec![
        json!({ "id": "docker", "label": "Start Docker", "weight": 0.5, "done": docker }),
        json!({ "id": "image", "label": "Download the vLLM image (about 9 GB)", "weight": 9.0, "done": image }),
        json!({ "id": "containers", "label": "Start the containers", "weight": 0.2, "done": vllm_up && shim_up }),
        json!({ "id": "weights", "label": "Download H2O-Lightning-4B weights (9.1 GB)", "weight": 9.1,
                "done": loaded || weights >= H2O_WEIGHT_BYTES, "progress": ratio(weights, H2O_WEIGHT_BYTES) }),
        json!({ "id": "load", "label": "Load the model on the GPU", "weight": 1.0, "done": loaded }),
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

fn compose_file(stack: Stack) -> Result<PathBuf, String> {
    let dir = home().join(stack.id());
    fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let file = dir.join("compose.yaml");
    fs::write(&file, stack.compose()).map_err(|e| format!("{}: {e}", file.display()))?;
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
            // Volumes stay, so a reinstall does not download the weights again.
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

/// Held while an action runs on a stack. Every jeff process takes the same lock file, the server and the CLI
/// alike, and the OS releases it when the holder exits.
pub struct Claim {
    stack: Stack,
    _lock: fs::File,
}

impl Claim {
    /// Claims the stack, or refuses while another action on it runs.
    pub fn new(stack: Stack) -> Result<Self, String> {
        Self::in_dir(stack, &home().join(stack.id()))
    }

    fn in_dir(stack: Stack, dir: &std::path::Path) -> Result<Self, String> {
        fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let path = dir.join("action.lock");
        let file = fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false)
            .open(&path)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        match file.try_lock() {
            Ok(()) => Ok(Self { stack, _lock: file }),
            Err(fs::TryLockError::WouldBlock) => Err(format!(
                "Another {} action is running. Wait for it to finish.",
                stack.name()
            )),
            Err(fs::TryLockError::Error(e)) => Err(format!("{}: {e}", path.display())),
        }
    }

    /// Runs the action on the claimed stack.
    pub fn run(
        &self,
        action: Action,
        inherit: bool,
        progress: &dyn Fn(&str),
    ) -> Result<String, String> {
        run(self.stack, action, inherit, progress)
    }
}

/// The running or last action of each stack, as the admin page shows it.
pub struct Tasks([Mutex<Value>; 2]);

impl Default for Tasks {
    fn default() -> Self {
        Self(std::array::from_fn(|_| {
            Mutex::new(json!({ "state": "idle" }))
        }))
    }
}

impl Tasks {
    /// The task of a stack; its `state` is idle, running, done or error.
    pub fn get(&self, stack: Stack) -> Value {
        self.0[stack as usize].lock().unwrap().clone()
    }

    /// Claims the stack and runs the action in the background.
    pub fn start(self: &Arc<Self>, stack: Stack, action: Action) -> Result<Value, String> {
        let claim = Claim::new(stack)?;
        *self.0[stack as usize].lock().unwrap() = json!({ "state": "running" });
        let tasks = self.clone();
        std::thread::spawn(move || {
            let task = &tasks.0[stack as usize];
            let progress =
                |step: &str| *task.lock().unwrap() = json!({ "state": "running", "step": step });
            let result = claim.run(action, false, &progress);
            // Released under the task lock: a start right after it is not refused, and its `running` cannot be
            // overwritten by this result.
            let mut task = task.lock().unwrap();
            drop(claim);
            *task = match result {
                Ok(log) => json!({ "state": "done", "log": log }),
                Err(e) => json!({ "state": "error", "error": e }),
            };
        });
        Ok(self.get(stack))
    }
}

/// Runs `docker compose` for a stack. `inherit` streams progress to this terminal; otherwise output is captured.
fn run(
    stack: Stack,
    action: Action,
    inherit: bool,
    progress: &dyn Fn(&str),
) -> Result<String, String> {
    ensure_docker(progress)?;
    let (name, port) = (stack.name(), stack.port());
    if matches!(action, Action::Install) && !port_free(port) && !stack.running() {
        return Err(format!(
            "Port {port} is taken by another program, perhaps a {name} server started by hand. Stop it and press Install again."
        ));
    }
    // Each stack reserves most of the GPU memory, so a second one would start and then fail to load its model.
    if let Some(other) = Stack::ALL
        .into_iter()
        .find(|s| matches!(action, Action::Install) && *s != stack && s.running())
    {
        return Err(format!(
            "{} is running and holds the GPU. Uninstall it, then install {name}.",
            other.name()
        ));
    }
    progress(&match action {
        Action::Install => {
            format!("Pulling images and starting {name}… the first pull is about 10 GB.")
        }
        Action::Remove => format!("Removing {name} containers…"),
    });
    let file = compose_file(stack)?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_claimed_stack_refuses_a_second_claim() {
        let dir = env::temp_dir().join(format!("jeff-claim-{}", std::process::id()));
        let first = Claim::in_dir(Stack::Clm, &dir).unwrap();
        let refused = Claim::in_dir(Stack::Clm, &dir).err().unwrap();
        assert_eq!(
            refused,
            "Another CLM action is running. Wait for it to finish."
        );
        drop(first);
        assert!(Claim::in_dir(Stack::Clm, &dir).is_ok());
        fs::remove_dir_all(&dir).ok();
    }
}
