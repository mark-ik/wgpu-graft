use std::path::{Path, PathBuf};

use url::Url;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RenderPath {
    GpuImport,
    CpuReadback,
}

impl RenderPath {
    fn as_str(self) -> &'static str {
        match self {
            Self::GpuImport => "GPU import",
            Self::CpuReadback => "CPU readback",
        }
    }
}

#[derive(Clone, Debug)]
pub struct DemoStatus {
    render_path: RenderPath,
    backend: Option<String>,
    frame_size: Option<(u32, u32)>,
    last_error: Option<String>,
}

impl DemoStatus {
    pub fn new(render_path: RenderPath) -> Self {
        Self {
            render_path,
            backend: None,
            frame_size: None,
            last_error: None,
        }
    }

    pub fn with_backend(mut self, backend: impl Into<String>) -> Self {
        self.backend = Some(backend.into());
        self
    }

    pub fn set_frame(&mut self, render_path: RenderPath, width: u32, height: u32) {
        self.render_path = render_path;
        self.frame_size = Some((width, height));
        if render_path == RenderPath::GpuImport {
            self.last_error = None;
        }
    }

    pub fn set_fallback_error(&mut self, error: impl ToString) {
        self.render_path = RenderPath::CpuReadback;
        self.last_error = Some(error.to_string());
    }

    pub fn summary(&self) -> String {
        let mut parts = vec![self.render_path.as_str().to_string()];
        if let Some(backend) = &self.backend {
            parts.push(format!("backend: {backend}"));
        }
        if let Some((width, height)) = self.frame_size {
            parts.push(format!("frame: {width}x{height}"));
        }
        if let Some(error) = &self.last_error {
            parts.push(format!("fallback: {error}"));
        }
        parts.join(" | ")
    }
}

pub fn resolve_initial_url(manifest_dir: &str) -> Result<Url, String> {
    if let Some(argument) = std::env::args().nth(1) {
        return resolve_url_argument(&argument);
    }

    let fixture = fixture_path(manifest_dir, "animated.html");
    Url::from_file_path(&fixture).map_err(|_| format!("fixture not found: {}", fixture.display()))
}

pub fn resolve_url_argument(argument: &str) -> Result<Url, String> {
    if let Ok(url) = Url::parse(argument) {
        return Ok(url);
    }

    if let Ok(url) = Url::parse(&format!("https://{argument}")) {
        return Ok(url);
    }

    let candidate = Path::new(argument);
    let absolute = if candidate.is_absolute() {
        candidate.to_owned()
    } else {
        std::env::current_dir()
            .map_err(|error| error.to_string())?
            .join(candidate)
    };

    Url::from_file_path(&absolute)
        .map_err(|_| format!("not a valid URL or file path: {argument}"))
}

pub fn fixture_path(manifest_dir: &str, file_name: &str) -> PathBuf {
    PathBuf::from(manifest_dir)
        .join("fixtures")
        .join(file_name)
}