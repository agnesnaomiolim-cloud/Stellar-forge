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
pub const SCALE: u128 = 1_000_000_000;
pub fn initial_ema(rate: u64) -> u128 { (rate as u128) * SCALE }
pub fn compute_ema(alpha: u64, prev_ema: u128, new_rate: u64) -> u128 {
    let alpha = alpha as u128;
    let new_rate = new_rate as u128;
    (alpha * new_rate * SCALE + (SCALE - alpha) * prev_ema) / SCALE
}
pub fn ema_scaled_to_units(ema_scaled: u128) -> u64 {
    (ema_scaled / SCALE) as u64
}
pub fn apply_bump_factor(ema_scaled: u128, bump_factor: u64) -> u64 {
    let units = ema_scaled_to_units(ema_scaled);
    let bump = bump_factor as u128;
    ((units as u128) * bump / SCALE) as u64
}