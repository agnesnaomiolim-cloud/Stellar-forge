// Copyright 2024 Stellar-K8s Contributors
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
//! WebAssembly caching layer for the Stellar operator.
//
// This module provides an in-memory, concurrency-safe LRU cache for pre-compiled
// Wasmtime modules. It is designed to be shared across all worker threads of a
// single RPC node and to be invalidated instantaneously when a contract's underlying
// network state is upgraded or altered.

pub mod metrics;
pub mod eviction;
pub mod cache_layer;

pub use cache_layer::{CacheKey, CachedModule, WasmCache, WasmCacheConfig, WasmCacheError};
pub use eviction::{EvictionPolicy, LruEvictionPolicy};
pub use metrics::{CacheMetrics, CacheMetricsSnapshot};
