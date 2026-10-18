//! The target the Tier 1 backend generates code for: the host, with fixed settings (D2 of
//! `codegen-core`).

use crate::error::BackendError;
use cranelift_codegen::isa::OwnedTargetIsa;
use cranelift_codegen::settings::{self, Configurable};

/// The Cranelift settings of every Tier 1 compilation, as `(name, value)` pairs.
pub fn settings() -> Vec<(&'static str, &'static str)> {
    vec![
        // Tier 1 is the development tier: Cranelift's optimizer costs more compile time than
        // it saves run time there, and its egraph pass grows with the depth of the dominator
        // tree, so a body of many sequential branches compiles in quadratic time with it. Tier 2
        // optimizes.
        ("opt_level", "none"),
        // The IR verifier checks every function in debug builds of `prism`, which run the tests;
        // release builds skip it for compile time.
        (
            "enable_verifier",
            if cfg!(debug_assertions) {
                "true"
            } else {
                "false"
            },
        ),
        // Profilers, and later backtraces, walk frame pointers.
        ("preserve_frame_pointers", "true"),
        // A large frame always touches the guard page, so a stack overflow faults there.
        ("enable_probestack", "true"),
        ("probestack_strategy", "inline"),
        // `cranelift-jit` needs long-range calls of runtime functions, and position-independent
        // code exactly on x86-64, where it resolves symbols through a GOT; it rejects PIC on
        // every other architecture.
        ("use_colocated_libcalls", "false"),
        (
            "is_pic",
            if cfg!(target_arch = "x86_64") {
                "true"
            } else {
                "false"
            },
        ),
    ]
}

/// The number of instructions above which a function is compiled with the single-pass register
/// allocator ([`large_function_isa`]).
///
/// The default, backtracking allocator generates better code but grows faster than the
/// function: with the depth of its dominator tree, as in a body of thousands of sequential
/// branches, and with the values live at once, as at a call of thousands of arguments. Below
/// the threshold that costs at most tens of milliseconds per function; above it, the
/// single-pass allocator keeps compilation linear, for code that runs about half as fast.
pub const LARGE_FUNCTION_INSTRUCTIONS: usize = 2048;

/// The host ISA with [`settings`].
pub fn isa() -> Result<OwnedTargetIsa, BackendError> {
    isa_with(&[])
}

/// The host ISA with [`settings`] and the single-pass register allocator, for the functions
/// larger than [`LARGE_FUNCTION_INSTRUCTIONS`].
pub fn large_function_isa() -> Result<OwnedTargetIsa, BackendError> {
    isa_with(&[("regalloc_algorithm", "single_pass")])
}

/// Whether `function` is compiled with [`large_function_isa`].
pub fn is_large(function: &cranelift_codegen::ir::Function) -> bool {
    let layout = &function.layout;
    layout
        .blocks()
        .map(|block| layout.block_insts(block).count())
        .sum::<usize>()
        > LARGE_FUNCTION_INSTRUCTIONS
}

/// The host ISA with [`settings`], then `overrides`.
fn isa_with(overrides: &[(&'static str, &'static str)]) -> Result<OwnedTargetIsa, BackendError> {
    let mut flags = settings::builder();
    for (name, value) in settings().into_iter().chain(overrides.iter().copied()) {
        flags
            .set(name, value)
            .map_err(|error| BackendError::new(format!("setting {}: {}", name, error)))?;
    }
    let builder = cranelift_native::builder()
        .map_err(|message| BackendError::new(format!("unsupported host: {}", message)))?;
    builder
        .finish(settings::Flags::new(flags))
        .map_err(|error| BackendError::new(format!("building the host ISA: {}", error)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_host_isa_has_the_fixed_settings() {
        let isa = isa().expect("the host is supported");
        let flags = isa.flags();
        assert_eq!(flags.opt_level(), settings::OptLevel::None);
        assert!(flags.preserve_frame_pointers());
        assert!(flags.enable_probestack());
        assert_eq!(flags.is_pic(), cfg!(target_arch = "x86_64"));
        assert!(!flags.use_colocated_libcalls());
        assert_eq!(
            flags.regalloc_algorithm(),
            settings::RegallocAlgorithm::Backtracking
        );
        let large = large_function_isa().expect("the host is supported");
        assert_eq!(
            large.flags().regalloc_algorithm(),
            settings::RegallocAlgorithm::SinglePass
        );
    }
}
