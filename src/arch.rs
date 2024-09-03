use std::str::FromStr;

#[derive(Debug, Clone, Copy)]
pub enum Arch {
    Arm64,
    Amd64,
}

impl Arch {
    /// Architecture as returned by `uname -m`.
    pub fn as_uname_m(&self) -> &str {
        match self {
            Arch::Arm64 => "aarch64",
            Arch::Amd64 => "x86_64",
        }
    }

    /// Architecture as returned by `dpkg --print-architecture`.
    pub fn as_dpkg(&self) -> &str {
        match self {
            Arch::Arm64 => "arm64",
            Arch::Amd64 => "amd64",
        }
    }
}

impl FromStr for Arch {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "arm64" | "aarch64" => Ok(Self::Arm64),
            "amd64" | "x86_64" => Ok(Self::Amd64),
            s => Err(anyhow::format_err!("Unknown arch: {s}")),
        }
    }
}
