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
use soroban_sdk::Env;

/// Calculates the required reserve token amount for minting `amount` of continuous tokens.
pub fn calculate_purchase_return(
    total_supply: i128,
    reserve_balance: i128,
    reserve_ratio: u32,
    amount: i128,
) -> i128 {
    if total_supply == 0 {
        return amount; // Base price of 1 when supply is 0. 
    }
    
    if reserve_ratio == 10000 {
        // Reserve Ratio 100%: Constant price. Cost = amount * (ReserveBalance / TotalSupply)
        return amount * reserve_balance / total_supply;
    }
    
    // Linear bonding curve approximation: ReserveRatio = 50% (5000)
    if reserve_ratio == 5000 {
        let ts = total_supply;
        let ts_plus_amount = total_supply + amount;
        
        let term1 = (ts_plus_amount * ts_plus_amount) / ts;
        let term2 = ts;
        
        return (reserve_balance * (term1 - term2)) / ts;
    }

    // Default fallback
    let ts = total_supply;
    let ts_plus_amount = total_supply + amount;
    (reserve_balance * (ts_plus_amount - ts)) / ts
}

/// Calculates the reserve token return for selling `amount` of continuous tokens.
pub fn calculate_sale_return(
    total_supply: i128,
    reserve_balance: i128,
    reserve_ratio: u32,
    amount: i128,
) -> i128 {
    if total_supply == 0 || amount == 0 {
        return 0;
    }
    
    if amount == total_supply {
        return reserve_balance;
    }

    if reserve_ratio == 10000 {
        return amount * reserve_balance / total_supply;
    }
    
    if reserve_ratio == 5000 {
        let ts = total_supply;
        let ts_minus_amount = total_supply - amount;
        
        let term1 = ts;
        let term2 = (ts_minus_amount * ts_minus_amount) / ts;
        
        return (reserve_balance * (term1 - term2)) / ts;
    }

    // Default fallback
    let ts = total_supply;
    let ts_minus_amount = total_supply - amount;
    (reserve_balance * (ts - ts_minus_amount)) / ts
}