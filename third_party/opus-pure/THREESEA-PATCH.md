# opus-pure 0.2.2, vendored

Pristine crates.io 0.2.2 (BSD-3-Clause) except:

- every `std::arch::x86_64::` path became `crate::simd_arch::`, an alias (end of `src/lib.rs`)
  for `std::arch::x86` on 32-bit x86 and `std::arch::x86_64` on x86_64. Upstream gates several SIMD functions with
  `any(target_arch = "x86", target_arch = "x86_64")` but imports only the 64-bit module, so the
  crate doesn't build for i686 (our Sailfish OS emulator target). Phones (aarch64, armv7hl) are
  unaffected.
- `benches/`, `examples/`, `docs/` removed.

Drop this copy once upstream builds for i686.
