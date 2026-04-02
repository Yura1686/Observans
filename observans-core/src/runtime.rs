use std::ffi::OsString;
use std::path::{Path, PathBuf};

pub fn resolve_ffmpeg_binary(
    env_override: Option<OsString>,
    current_exe: Option<PathBuf>,
    platform_name: &str,
) -> PathBuf {
    if let Some(path) = env_override.filter(|value| !value.is_empty()) {
        return PathBuf::from(path);
    }

    if let Some(executable) = current_exe {
        if let Some(candidate) = bundled_ffmpeg_candidates(&executable, platform_name)
            .into_iter()
            .find(|candidate| candidate.is_file())
        {
            return candidate;
        }
    }

    Path::new(ffmpeg_executable_name(platform_name)).to_path_buf()
}

pub fn resolve_ffmpeg_for_current_process(platform_name: &str) -> PathBuf {
    resolve_ffmpeg_binary(
        std::env::var_os("OBSERVANS_FFMPEG"),
        std::env::current_exe().ok(),
        platform_name,
    )
}

pub fn bundled_ffmpeg_path(executable: &Path, platform_name: &str) -> PathBuf {
    bundled_ffmpeg_candidates(executable, platform_name)
        .into_iter()
        .next()
        .unwrap_or_else(|| {
            Path::new(".")
                .join("_observans_runtime")
                .join("ffmpeg")
                .join("bin")
                .join(ffmpeg_executable_name(platform_name))
        })
}

fn bundled_ffmpeg_candidates(executable: &Path, platform_name: &str) -> Vec<PathBuf> {
    executable
        .parent()
        .into_iter()
        .flat_map(|directory| directory.ancestors())
        .map(|directory| {
            directory
                .join("_observans_runtime")
                .join("ffmpeg")
                .join("bin")
                .join(ffmpeg_executable_name(platform_name))
        })
        .collect()
}

pub fn ffmpeg_executable_name(platform_name: &str) -> &'static str {
    if platform_name == "windows" {
        "ffmpeg.exe"
    } else {
        "ffmpeg"
    }
}
