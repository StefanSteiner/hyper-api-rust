// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Build script for the napi-rs Node.js bindings: delegates to `napi_build::setup`.

extern crate napi_build;

fn main() {
    napi_build::setup();
}
