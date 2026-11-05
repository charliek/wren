use std::error::Error;
use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use iced::window::Screenshot;
use wren_core::theme::{self, Theme};

#[derive(Debug, Clone)]
pub struct Request {
    pub theme: Theme,
    output: PathBuf,
    completed: Arc<AtomicBool>,
}

impl Request {
    pub fn parse(args: &[OsString], test_mode: Option<&OsStr>) -> Result<Option<Self>, String> {
        if args.is_empty() {
            return Ok(None);
        }
        if args.len() != 3 || args[0] != "--smoke-test" {
            return Err("usage: wren --smoke-test <dark|light> <absolute-output.png>".into());
        }
        if test_mode != Some(OsStr::new("1")) {
            return Err("smoke capture requires WREN_TEST_MODE=1".into());
        }
        let theme = args[1]
            .to_str()
            .and_then(theme::builtin)
            .ok_or("unknown smoke theme")?;
        let output = PathBuf::from(&args[2]);
        if !output.is_absolute() {
            return Err("smoke output path must be absolute".into());
        }
        Ok(Some(Self {
            theme,
            output,
            completed: Arc::new(AtomicBool::new(false)),
        }))
    }

    pub fn completion(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.completed)
    }

    pub fn save(&self, capture: &Screenshot) -> Result<(), Box<dyn Error>> {
        let width = capture.size.width;
        let height = capture.size.height;
        let expected = (width as usize)
            .checked_mul(height as usize)
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or("screenshot dimensions overflow")?;
        if width == 0 || height == 0 || capture.rgba.len() != expected {
            return Err("invalid screenshot dimensions or pixel buffer".into());
        }

        let mut png_bytes = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut png_bytes, width, height);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder.set_compression(png::Compression::Fast);
            encoder.set_filter(png::Filter::NoFilter);
            let mut writer = encoder.write_header()?;
            writer.write_image_data(&capture.rgba)?;
            writer.finish()?;
        }

        let mut output = File::create_new(&self.output)?;
        output.write_all(&png_bytes)?;
        output.flush()?;
        self.completed.store(true, Ordering::SeqCst);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(theme: &str, output: &str) -> Vec<OsString> {
        ["--smoke-test", theme, output]
            .into_iter()
            .map(OsString::from)
            .collect()
    }

    #[test]
    fn ordinary_launch_never_enables_capture() {
        assert!(Request::parse(&[], None).unwrap().is_none());
        assert!(Request::parse(&[], Some(OsStr::new("1")))
            .unwrap()
            .is_none());
    }

    #[test]
    fn capture_requires_explicit_runtime_opt_in() {
        assert!(Request::parse(&args("dark", "/tmp/wren.png"), None).is_err());
        assert!(Request::parse(&args("dark", "/tmp/wren.png"), Some(OsStr::new("0"))).is_err());
        let request = Request::parse(&args("dark", "/tmp/wren.png"), Some(OsStr::new("1")))
            .unwrap()
            .unwrap();
        assert_eq!(request.theme, theme::DARK);
        assert!(!request.completion().load(Ordering::SeqCst));
    }

    #[test]
    fn invalid_theme_path_and_arguments_are_rejected() {
        for invalid in [
            args("missing", "/tmp/wren.png"),
            args("light", "relative.png"),
            vec![OsString::from("--unknown")],
        ] {
            assert!(Request::parse(&invalid, Some(OsStr::new("1"))).is_err());
        }
    }
}
