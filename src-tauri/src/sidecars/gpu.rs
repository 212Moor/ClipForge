use serde::{Deserialize, Serialize};
use specta::Type;
use std::path::Path;
use std::process::Command;
use std::sync::OnceLock;

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum WhisperVariant {
    Cuda,
    Vulkan,
    Cpu,
}

impl WhisperVariant {
    pub const ALL: [WhisperVariant; 3] = [Self::Cuda, Self::Vulkan, Self::Cpu];

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Cuda => "cuda",
            Self::Vulkan => "vulkan",
            Self::Cpu => "cpu",
        }
    }
}

/// Capacités matérielles détectées (indépendantes de ce que propose le manifeste).
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct GpuCapabilities {
    pub adapters: Vec<String>,
    pub has_cuda: bool,
    pub has_vulkan: bool,
}

static GPU_CACHE: OnceLock<GpuCapabilities> = OnceLock::new();

/// Détecte les GPU Windows et la présence des runtimes CUDA/Vulkan.
/// Bloquant (appelle PowerShell) : à utiliser dans `blocking(...)`. Résultat mis en cache pour la session.
pub fn detect_gpu_capabilities() -> GpuCapabilities {
    GPU_CACHE
        .get_or_init(|| {
            let adapters = query_windows_gpus();
            let has_nvidia = adapters
                .iter()
                .any(|name| name.to_ascii_lowercase().contains("nvidia"));

            let has_cuda = has_nvidia && (has_system_dll("nvcuda.dll") || nvidia_smi_available());
            let has_vulkan = !adapters.is_empty() && has_system_dll("vulkan-1.dll");

            tracing::info!(?adapters, has_cuda, has_vulkan, "Capacités GPU détectées");

            GpuCapabilities {
                adapters,
                has_cuda,
                has_vulkan,
            }
        })
        .clone()
}

/// Meilleure variante de whisper.cpp pour ce matériel, parmi celles réellement proposées par le manifeste.
pub fn recommend_variant(gpu: &GpuCapabilities, available: &[WhisperVariant]) -> WhisperVariant {
    if gpu.has_cuda && available.contains(&WhisperVariant::Cuda) {
        WhisperVariant::Cuda
    } else if gpu.has_vulkan && available.contains(&WhisperVariant::Vulkan) {
        WhisperVariant::Vulkan
    } else {
        WhisperVariant::Cpu
    }
}

fn has_system_dll(dll_name: &str) -> bool {
    let sys_root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
    Path::new(&sys_root).join("System32").join(dll_name).exists()
}

fn hidden_command(program: &str) -> Command {
    let mut cmd = Command::new(program);
    #[cfg(target_os = "windows")]
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd
}

fn nvidia_smi_available() -> bool {
    hidden_command("nvidia-smi")
        .args(["--query-gpu=name", "--format=csv,noheader"])
        .output()
        .map(|out| out.status.success())
        .unwrap_or(false)
}

fn query_windows_gpus() -> Vec<String> {
    #[cfg(target_os = "windows")]
    {
        // Pas de guillemets dans la commande : un nom de GPU par ligne.
        let output = hidden_command("powershell")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "Get-CimInstance Win32_VideoController | ForEach-Object { $_.Name }",
            ])
            .output();

        if let Ok(output) = output {
            if output.status.success() {
                return String::from_utf8_lossy(&output.stdout)
                    .lines()
                    .map(str::trim)
                    .filter(|line| !line.is_empty())
                    .map(ToOwned::to_owned)
                    .collect();
            }
        }
    }
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gpu(has_cuda: bool, has_vulkan: bool) -> GpuCapabilities {
        GpuCapabilities {
            adapters: vec![],
            has_cuda,
            has_vulkan,
        }
    }

    #[test]
    fn recommends_cuda_only_when_available_in_manifest() {
        let all = [WhisperVariant::Cuda, WhisperVariant::Cpu];
        assert_eq!(recommend_variant(&gpu(true, true), &all), WhisperVariant::Cuda);
        assert_eq!(
            recommend_variant(&gpu(true, true), &[WhisperVariant::Cpu]),
            WhisperVariant::Cpu
        );
    }

    #[test]
    fn falls_back_to_cpu_without_gpu() {
        let all = [WhisperVariant::Cuda, WhisperVariant::Vulkan, WhisperVariant::Cpu];
        assert_eq!(recommend_variant(&gpu(false, false), &all), WhisperVariant::Cpu);
        assert_eq!(recommend_variant(&gpu(false, true), &all), WhisperVariant::Vulkan);
    }
}
