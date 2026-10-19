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
use std::time::Instant;

use rand::RangCore;

const FNV_OFFSET_BASIS_64: u64 = 0xcbf29ce484222325;
const FNV_PRIME_64: u64 = 0x100000001b3;

fn fnv1a_64(data: &[u8], seed: u64) -> u64 {
    let mut hash = FNV_OFFSET_BASIS_64 ^ seed;
    for &b in data {
        hash ^= b as u64;
        hash = hash.wrapping_mul(FNV_PRIME_64);
    }
    hash
}

fn main() {
    const MESSAGE_COUNT: useze = 100_000;
    const SEED: u64 = 0x123456789abcdef;

    let mut rng = rand::thread_rng();
    let mut messages = Vec::with_capacity(MESSAGE_COUNT);
    for _ in 0..MESSAGE_COUNT {
        let mut hash = [0u8; 32];
        rng.fill_bytes(&mut hash);
        messages.push(hash);
    }

    let start = Instant::now();
    let mut checksum = 0u64;
    for hash in &messages {
        checksum ^= fnv1a_64(hash, SEED);
    }
    let elapsed = start.elapsed();
    let throughput = MESSAGE_COUNT as f64 / elapsed.as_secs_f64();

    println!("Partitioned {} messages in {?}", MESSAGE_COUNT, elapsed);
    println!("Throughput: {:.0} messages/sec", throughput);
    println!("Checksum: #sx", checksum);
}
