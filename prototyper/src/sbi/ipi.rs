use core::sync::atomic::{AtomicPtr, Ordering::Relaxed};
use rustsbi::SbiRet;

use crate::board::SBI_IMPL;
use crate::riscv_spec::{current_hartid, stimecmp};
use crate::sbi::extensions::{hart_extension_probe, Extension};
use crate::sbi::hsm::remote_hsm;
use crate::sbi::rfence;
use crate::sbi::trap;
use crate::sbi::trap_stack::ROOT_STACK;

/// IPI type for supervisor software interrupt.
pub(crate) const IPI_TYPE_SSOFT: u8 = 1 << 0;
/// IPI type for memory fence operations.
pub(crate) const IPI_TYPE_FENCE: u8 = 1 << 1;

/// Trait defining interface for inter-processor interrupt device
#[allow(unused)]
pub trait IpiDevice {
    /// Read machine time value.
    fn read_mtime(&self) -> u64;
    /// Write machine time value.
    fn write_mtime(&self, val: u64);
    /// Read machine timer compare value for given hart.
    fn read_mtimecmp(&self, hart_idx: usize) -> u64;
    /// Write machine timer compare value for given hart.
    fn write_mtimecmp(&self, hart_idx: usize, val: u64);
    /// Read machine software interrupt pending bit for given hart.
    fn read_msip(&self, hart_idx: usize) -> bool;
    /// Set machine software interrupt pending bit for given hart.
    fn set_msip(&self, hart_idx: usize);
    /// Clear machine software interrupt pending bit for given hart.
    fn clear_msip(&self, hart_idx: usize);
}

/// SBI IPI implementation.
pub struct SbiIpi<'a, T: IpiDevice> {
    /// Reference to atomic pointer to IPI device.
    pub ipi_dev: &'a AtomicPtr<T>,
    /// Maximum hart ID in the system
    pub max_hart_id: usize,
}

impl<'a, T: IpiDevice> rustsbi::Timer for SbiIpi<'a, T> {
    /// Set timer value for current hart.
    #[inline]
    fn set_timer(&self, stime_value: u64) {
        let hart_id = current_hartid();
        let uses_sstc = hart_extension_probe(hart_id, Extension::Sstc);

        // Set timer value based on extension support.
        if uses_sstc {
            stimecmp::set(stime_value);
        } else {
            self.write_mtimecmp(hart_id, stime_value);
            unsafe {
                riscv::register::mip::clear_stimer();
            }
        }
        // Enable machine timer interrupt.
        unsafe {
            riscv::register::mie::set_mtimer();
        }
    }
}

impl<'a, T: IpiDevice> rustsbi::Ipi for SbiIpi<'a, T> {
    /// Send IPI to specified harts.
    #[inline]
    fn send_ipi(&self, hart_mask: rustsbi::HartMask) -> SbiRet {
        let ipi_dev = unsafe { &*self.ipi_dev.load(Relaxed) };

        for hart_id in 0..=self.max_hart_id {
            if !hart_mask.has_bit(hart_id) {
                continue;
            }

            let Some(hsm) = remote_hsm(hart_id) else {
                continue;
            };

            if !hsm.allow_ipi() {
                continue;
            }

            if set_ipi_type(hart_id, IPI_TYPE_SSOFT) == 0 {
                ipi_dev.set_msip(hart_id);
            }
        }

        SbiRet::success(0)
    }
}

impl<'a, T: IpiDevice> SbiIpi<'a, T> {
    /// Create new SBI IPI instance.
    #[inline]
    pub fn new(ipi_dev: &'a AtomicPtr<T>, max_hart_id: usize) -> Self {
        Self {
            ipi_dev,
            max_hart_id,
        }
    }

    /// Send IPI for remote fence operation.
    pub fn send_ipi_by_fence(
        &self,
        hart_mask: rustsbi::HartMask,
        ctx: rfence::RFenceContext,
    ) -> SbiRet {
        let current_hart = current_hartid();
        let ipi_dev = unsafe { &*self.ipi_dev.load(Relaxed) };

        // Send fence operations to target harts
        for hart_id in 0..=self.max_hart_id {
            if !hart_mask.has_bit(hart_id) {
                continue;
            }

            let Some(hsm) = remote_hsm(hart_id) else {
                continue;
            };

            if !hsm.allow_ipi() {
                continue;
            }

            if let Some(remote) = rfence::remote_rfence(hart_id) {
                remote.set(ctx);
                if let Some(local) = rfence::local_rfence() {
                    local.add();
                }
                if hart_id != current_hart {
                    let old_ipi_type = set_ipi_type(hart_id, IPI_TYPE_FENCE);
                    if old_ipi_type == 0 {
                        ipi_dev.set_msip(hart_id);
                    }
                }
            }
        }

        // Wait for all fence operations to complete
        while !rfence::local_rfence().unwrap().is_sync() {
            trap::rfence_single_handler();
        }

        SbiRet::success(0)
    }

    /// Get lower 32 bits of machine time.
    #[inline]
    pub fn get_time(&self) -> usize {
        unsafe { (*self.ipi_dev.load(Relaxed)).read_mtime() as usize }
    }

    /// Get upper 32 bits of machine time.
    #[inline]
    pub fn get_timeh(&self) -> usize {
        unsafe { ((*self.ipi_dev.load(Relaxed)).read_mtime() >> 32) as usize }
    }

    /// Set machine software interrupt pending for hart.
    #[inline]
    pub fn set_msip(&self, hart_idx: usize) {
        unsafe { (*self.ipi_dev.load(Relaxed)).set_msip(hart_idx) }
    }

    /// Clear machine software interrupt pending for hart.
    #[inline]
    pub fn clear_msip(&self, hart_idx: usize) {
        unsafe { (*self.ipi_dev.load(Relaxed)).clear_msip(hart_idx) }
    }

    /// Write machine timer compare value for hart.
    #[inline]
    pub fn write_mtimecmp(&self, hart_idx: usize, val: u64) {
        unsafe { (*self.ipi_dev.load(Relaxed)).write_mtimecmp(hart_idx, val) }
    }

    /// Clear all pending interrupts for current hart.
    #[inline]
    pub fn clear(&self) {
        let hart_id = current_hartid();
        // Load ipi_dev once instead of twice
        let ipi_dev = unsafe { &*self.ipi_dev.load(Relaxed) };
        ipi_dev.clear_msip(hart_id);
        ipi_dev.write_mtimecmp(hart_id, u64::MAX);
    }
}

/// Set IPI type for specified hart.
pub fn set_ipi_type(hart_id: usize, event_id: u8) -> u8 {
    unsafe {
        ROOT_STACK
            .get_unchecked_mut(hart_id)
            .hart_context()
            .ipi_type
            .fetch_or(event_id, Relaxed)
    }
}

/// Get and reset IPI type for current hart.
pub fn get_and_reset_ipi_type() -> u8 {
    unsafe {
        ROOT_STACK
            .get_unchecked_mut(current_hartid())
            .hart_context()
            .ipi_type
            .swap(0, Relaxed)
    }
}

/// Clear machine software interrupt pending for current hart.
#[inline]
pub fn clear_msip() {
    match unsafe { SBI_IMPL.as_ptr().as_ref().and_then(|sbi| sbi.ipi.as_ref()) } {
        Some(ipi) => ipi.clear_msip(current_hartid()),
        None => error!("SBI or IPI device not initialized"),
    }
}

/// Clear machine timer interrupt for current hart.
#[inline]
pub fn clear_mtime() {
    match unsafe { SBI_IMPL.as_ptr().as_ref().and_then(|sbi| sbi.ipi.as_ref()) } {
        Some(ipi) => ipi.write_mtimecmp(current_hartid(), u64::MAX),
        None => error!("SBI or IPI device not initialized"),
    }
}

/// Clear all pending interrupts for current hart.
#[inline]
pub fn clear_all() {
    match unsafe { SBI_IMPL.as_ptr().as_ref().and_then(|sbi| sbi.ipi.as_ref()) } {
        Some(ipi) => ipi.clear(),
        None => error!("SBI or IPI device not initialized"),
    }
}
