# Changelog

All notable changes to this project will be documented in this file.
The project adheres to [Semantic Versioning](http://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Support upcasting and downcasting of `Resource`s.
- Support expressions in `link_name` / `export_name` attributes, such as 
  `#[export_name = concat("prefix_", stringify!($name))]` for use in macros (`$name`
  is a macro variable). Previously, only string literals were supported.
- Support re-exporting the crate by adding an optional `crate` parameter
  to the `#[externref]` attribute, e.g. `#[externref(crate = "other_crate::_externref")]`
  where `other_crate` defines `pub use externref as _externref`.

### Fixed

- Fix an incorrect conditional compilation attribute for a tracing event
  in the processor module.
- Fix / document miscompilation resulting from optimization tools inlining
  an `externref`-operation function. The processor now returns an error
  if it encounters such an inlined function, and the docs mention how to avoid
  inlining (do not run WASM optimization tools before the `externref` processor).

## 0.1.0 - 2022-10-29

The initial release of `externref`.
