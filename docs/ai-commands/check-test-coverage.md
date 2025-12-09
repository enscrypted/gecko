# Command: check-test-coverage

## Description
Identifies missing test coverage for new or modified code. Scans public functions and checks for corresponding tests.

## Triggers

### Automatic
- After implementing a new feature
- After fixing a bug
- Before creating a PR

### Manual
- User says: "run check-test-coverage"
- User says: "do we have tests for this?"

## Prerequisites
- Code files have been created or modified
- Test modules exist in crates

## Steps

1. Identify target files from current session

2. For each Rust file:
   - Find public functions (`pub fn`)
   - Check for corresponding `#[test]` in same file or `tests/` folder
   - Note any `#[ignore]` tests that might be relevant

3. For each TypeScript file:
   - Check for `.test.ts` or `.spec.ts` file
   - Note if component has test coverage

4. Generate report:
   - Functions missing tests
   - Functions with tests
   - Ignored tests that might need attention

5. Offer to generate skeleton tests

## Output
- Console report of coverage gaps
- Option to generate skeleton tests

## Example

```
Test Coverage Report
====================

crates/gecko_dsp/src/eq.rs
  pub fn new() - COVERED (test_default_config_is_flat)
  pub fn set_band_gain() - COVERED (test_gain_clamping)
  pub fn process_sample() - COVERED (test_eq_steady_state_response)
  pub fn process_interleaved() - COVERED (test_interleaved_processing)
  pub fn reset() - COVERED (test_reset_doesnt_panic)

crates/gecko_core/src/engine.rs
  pub fn new() - COVERED
  pub fn start() - COVERED (ignored: requires audio hardware)
  pub fn set_band_gain() - MISSING TEST
  pub fn list_devices() - COVERED (ignored: requires audio hardware)

Summary:
- 15 public functions
- 12 covered (80%)
- 3 missing tests
- 2 hardware-dependent (ignored)

Would you like me to generate skeleton tests for the missing functions?
```
