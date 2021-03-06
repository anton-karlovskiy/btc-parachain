//! Runtime API definition for the SLA Module.

#![cfg_attr(not(feature = "std"), no_std)]

use codec::Codec;
use frame_support::dispatch::DispatchError;
use module_exchange_rate_oracle_rpc_runtime_api::BalanceWrapper;

sp_api::decl_runtime_apis! {
    pub trait SlaApi<AccountId, DOT> where
        AccountId: Codec,
        DOT: Codec
    {
        /// Calculate the slashed amount for the given vault
        fn calculate_slashed_amount(vault_id: AccountId, stake: BalanceWrapper<DOT>) -> Result<BalanceWrapper<DOT>, DispatchError>;
    }
}
