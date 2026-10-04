//! Exact Windows SCM launch contract, not an SCM registration adapter.
//! An ordinary console Vessel must never be registered as a service executable.
use anyhow::{Result, ensure};
use std::path::Path;
const NAME: &str = "VoyageVessel";

pub fn command_line(bin: &Path, state: &Path, service_host_supported: bool) -> Result<String> {
    ensure!(
        service_host_supported,
        "Vessel lacks native SCM service-host protocol; refusing service registration"
    );
    let binary = bin.join("vessel.exe");
    let voyage = bin.join("voyage.exe");
    for path in [&binary, &voyage, state] {
        let text = path
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("Native service path must be Unicode"))?;
        ensure!(
            !text.is_empty() && !text.chars().any(char::is_control),
            "Unsafe native service path"
        );
        // Windows absolute paths cannot be tested using host Path::is_absolute
        // in portable fixtures. Reject device/UNC roots and relative drive paths.
        let bytes = text.as_bytes();
        ensure!(
            bytes.len() > 3
                && bytes[0].is_ascii_alphabetic()
                && bytes[1] == b':'
                && matches!(bytes[2], b'\\' | b'/'),
            "SCM paths require absolute local drive paths"
        );
    }
    Ok(format!(
        "{} --windows-service {} --directory {} --voyage-binary {}",
        quote(binary.to_str().unwrap()),
        quote(NAME),
        quote(state.to_str().unwrap()),
        quote(voyage.to_str().unwrap())
    ))
}
// CommandLineToArgvW/CRT quoting: double backslashes before quote and terminator.
fn quote(value: &str) -> String {
    let mut result = String::from("\"");
    let mut slashes = 0;
    for c in value.chars() {
        if c == '\\' {
            slashes += 1;
            continue;
        }
        if c == '"' {
            result.extend(std::iter::repeat_n('\\', slashes * 2 + 1));
        } else {
            result.extend(std::iter::repeat_n('\\', slashes));
        }
        result.push(c);
        slashes = 0;
    }
    result.extend(std::iter::repeat_n('\\', slashes * 2));
    result.push('"');
    result
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn no_console_binary_service_registration() {
        assert!(
            command_line(
                Path::new(r"C:\Voyage\bin"),
                Path::new(r"C:\Voyage\state"),
                false
            )
            .is_err()
        );
    }
    #[test]
    fn quoting_preserves_spaces_backslashes_and_quotes() {
        assert_eq!(quote(r"C:\a b\"), "\"C:\\a b\\\\\"");
        assert_eq!(quote("a\"b"), "\"a\\\"b\"");
    }
    #[test]
    fn reject_relative_unc_and_controls() {
        for state in [r"C:relative", r"\\server\share", "C:\\unsafe\npath"] {
            assert!(command_line(Path::new(r"C:\Voyage\bin"), Path::new(state), true).is_err());
        }
    }
}
