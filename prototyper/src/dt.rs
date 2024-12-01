use serde::Deserialize;
use serde_device_tree::{
    buildin::{NodeSeq, Reg, StrSeq},
    Dtb, DtbPtr,
};

/// Root device tree structure containing system information.
#[derive(Deserialize)]
pub struct Tree<'a> {
    /// Optional model name string.
    pub model: Option<StrSeq<'a>>,
    /// Chosen node containing boot parameters.
    pub chosen: Chosen<'a>,
    /// CPU information.
    pub cpus: Cpus<'a>,
    /// System-on-chip components.
    pub soc: Soc<'a>,
}

/// Chosen node containing boot parameters.
#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Chosen<'a> {
    /// Path to stdout device.
    pub stdout_path: StrSeq<'a>,
}

/// CPU information container.
#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Cpus<'a> {
    /// Sequence of CPU nodes.
    pub cpu: NodeSeq<'a>,
}

/// Individual CPU node information.
#[derive(Deserialize, Debug)]
pub struct Cpu<'a> {
    /// RISC-V ISA extensions supported by this CPU.
    #[serde(rename = "riscv,isa-extensions")]
    pub isa: Option<StrSeq<'a>>,
    /// CPU register information.
    pub reg: Reg<'a>,
}

/// System-on-chip components.
#[derive(Deserialize, Debug)]
pub struct Soc<'a> {
    /// Serial (UART) device nodes.
    pub serial: Option<NodeSeq<'a>>,
    /// Test device nodes.
    pub test: Option<NodeSeq<'a>>,
    /// CLINT (Core Local Interruptor) nodes.
    pub clint: Option<NodeSeq<'a>>,
}

/// Generic device node information.
#[allow(unused)]
#[derive(Deserialize, Debug)]
pub struct Device<'a> {
    /// Device register information.
    pub reg: Reg<'a>,
}

/// Errors that can occur during device tree parsing.
pub enum ParseDeviceTreeError {
    /// Invalid device tree format.
    Format,
}

pub fn parse_device_tree(opaque: usize) -> Result<Dtb, ParseDeviceTreeError> {
    let Ok(ptr) = DtbPtr::from_raw(opaque as *mut _) else {
        return Err(ParseDeviceTreeError::Format);
    };
    let dtb = Dtb::from(ptr);
    Ok(dtb)
}
