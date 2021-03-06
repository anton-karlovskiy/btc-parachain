use crate::sp_api_hidden_includes_decl_storage::hidden_include::StorageValue;
use crate::{ext, Config, Error, Module};
use codec::{Decode, Encode, HasCompact};
use frame_support::traits::Currency;
use frame_support::{
    dispatch::{DispatchError, DispatchResult},
    ensure, StorageMap,
};
use sp_arithmetic::FixedPointNumber;
use sp_core::H256;
use sp_runtime::traits::CheckedAdd;
use sp_runtime::traits::CheckedSub;
use sp_std::collections::btree_set::BTreeSet;

#[cfg(test)]
use mocktopus::macros::mockable;

pub use bitcoin::{Address as BtcAddress, PublicKey as BtcPublicKey};

/// Storage version.
#[derive(Encode, Decode, Eq, PartialEq)]
pub enum Version {
    /// Initial version.
    V0,
    /// BtcAddress type with script format.
    V1,
}

pub(crate) type DOT<T> =
    <<T as collateral::Config>::DOT as Currency<<T as frame_system::Config>::AccountId>>::Balance;

pub(crate) type PolkaBTC<T> = <<T as treasury::Config>::PolkaBTC as Currency<
    <T as frame_system::Config>::AccountId,
>>::Balance;

pub(crate) type UnsignedFixedPoint<T> = <T as Config>::UnsignedFixedPoint;

pub(crate) type Inner<T> = <<T as Config>::UnsignedFixedPoint as FixedPointNumber>::Inner;

#[derive(Encode, Decode, Clone, PartialEq, Debug, Default)]
pub struct Wallet {
    // store all addresses for `report_vault_theft` checks
    pub addresses: BTreeSet<BtcAddress>,
    // we use this public key to generate new addresses
    pub public_key: BtcPublicKey,
}

impl Wallet {
    pub fn new(public_key: BtcPublicKey) -> Self {
        Self {
            addresses: BTreeSet::new(),
            public_key,
        }
    }

    pub fn has_btc_address(&self, address: &BtcAddress) -> bool {
        self.addresses.contains(address)
    }

    pub fn add_btc_address(&mut self, address: BtcAddress) {
        // TODO: add maximum or griefing collateral
        self.addresses.insert(address);
    }
}

#[derive(Encode, Decode, Clone, Copy, PartialEq, Debug)]
pub enum VaultStatus {
    /// Vault is active
    Active = 0,

    /// Vault has been liquidated
    Liquidated = 1,

    /// Vault theft has been reported
    CommittedTheft = 2,
}

impl Default for VaultStatus {
    fn default() -> Self {
        VaultStatus::Active
    }
}

#[derive(Encode, Decode, Default, Clone, PartialEq)]
#[cfg_attr(feature = "std", derive(Debug))]
pub struct Vault<AccountId, BlockNumber, PolkaBTC> {
    // Account identifier of the Vault
    pub id: AccountId,
    // Number of PolkaBTC tokens pending issue
    pub to_be_issued_tokens: PolkaBTC,
    // Number of issued PolkaBTC tokens
    pub issued_tokens: PolkaBTC,
    // Number of PolkaBTC tokens pending redeem
    pub to_be_redeemed_tokens: PolkaBTC,
    // Bitcoin address of this Vault (P2PKH, P2SH, P2PKH, P2WSH)
    pub wallet: Wallet,
    // Block height until which this Vault is banned from being
    // used for Issue, Redeem (except during automatic liquidation) and Replace .
    pub banned_until: Option<BlockNumber>,
    /// Current status of the vault
    pub status: VaultStatus,
}

#[derive(Encode, Decode, Default, Clone, PartialEq)]
#[cfg_attr(feature = "std", derive(Debug, serde::Serialize, serde::Deserialize))]
pub struct SystemVault<AccountId, PolkaBTC> {
    // Account identifier of the Vault
    pub id: AccountId,
    // Number of PolkaBTC tokens pending issue
    pub to_be_issued_tokens: PolkaBTC,
    // Number of issued PolkaBTC tokens
    pub issued_tokens: PolkaBTC,
    // Number of PolkaBTC tokens pending redeem
    pub to_be_redeemed_tokens: PolkaBTC,
}

impl<AccountId, BlockNumber, PolkaBTC: HasCompact + Default>
    Vault<AccountId, BlockNumber, PolkaBTC>
{
    pub(crate) fn new(
        id: AccountId,
        public_key: BtcPublicKey,
    ) -> Vault<AccountId, BlockNumber, PolkaBTC> {
        let wallet = Wallet::new(public_key);
        Vault {
            id,
            wallet,
            to_be_issued_tokens: Default::default(),
            issued_tokens: Default::default(),
            to_be_redeemed_tokens: Default::default(),
            banned_until: None,
            status: VaultStatus::Active,
        }
    }

    pub fn is_liquidated(&self) -> bool {
        matches!(self.status, VaultStatus::Liquidated)
    }
}

pub type DefaultVault<T> = Vault<
    <T as frame_system::Config>::AccountId,
    <T as frame_system::Config>::BlockNumber,
    PolkaBTC<T>,
>;

pub type DefaultSystemVault<T> = SystemVault<<T as frame_system::Config>::AccountId, PolkaBTC<T>>;

pub trait UpdatableVault<T: Config> {
    fn id(&self) -> T::AccountId;

    fn issued_tokens(&self) -> PolkaBTC<T>;

    fn force_issue_tokens(&mut self, tokens: PolkaBTC<T>) -> ();

    fn force_increase_to_be_issued(&mut self, tokens: PolkaBTC<T>) -> ();

    fn force_increase_to_be_redeemed(&mut self, tokens: PolkaBTC<T>) -> ();

    fn force_decrease_issued(&mut self, tokens: PolkaBTC<T>) -> ();

    fn force_decrease_to_be_issued(&mut self, tokens: PolkaBTC<T>) -> ();

    fn force_decrease_to_be_redeemed(&mut self, tokens: PolkaBTC<T>) -> ();

    fn decrease_issued(&mut self, tokens: PolkaBTC<T>) -> DispatchResult {
        let issued_tokens = self.issued_tokens();
        ensure!(
            issued_tokens >= tokens,
            Error::<T>::InsufficientTokensCommitted
        );
        Ok(self.force_decrease_issued(tokens))
    }
}

pub struct RichVault<T: Config> {
    pub(crate) data: DefaultVault<T>,
}

impl<T: Config> UpdatableVault<T> for RichVault<T> {
    fn id(&self) -> T::AccountId {
        self.data.id.clone()
    }

    fn issued_tokens(&self) -> PolkaBTC<T> {
        self.data.issued_tokens
    }

    fn force_issue_tokens(&mut self, tokens: PolkaBTC<T>) -> () {
        self.update(|v| v.issued_tokens += tokens)
    }

    fn force_increase_to_be_issued(&mut self, tokens: PolkaBTC<T>) -> () {
        self.update(|v| v.to_be_issued_tokens += tokens);
    }

    fn force_increase_to_be_redeemed(&mut self, tokens: PolkaBTC<T>) -> () {
        self.update(|v| v.to_be_redeemed_tokens += tokens);
    }

    fn force_decrease_issued(&mut self, tokens: PolkaBTC<T>) -> () {
        self.update(|v| v.issued_tokens -= tokens);
    }

    fn force_decrease_to_be_issued(&mut self, tokens: PolkaBTC<T>) -> () {
        self.update(|v| v.to_be_issued_tokens -= tokens);
    }

    fn force_decrease_to_be_redeemed(&mut self, tokens: PolkaBTC<T>) -> () {
        self.update(|v| v.to_be_redeemed_tokens -= tokens);
    }
}

#[cfg_attr(test, mockable)]
impl<T: Config> RichVault<T> {
    pub fn increase_collateral(&self, collateral: DOT<T>) -> DispatchResult {
        ext::collateral::lock::<T>(&self.data.id, collateral)
    }

    pub fn withdraw_collateral(&self, collateral: DOT<T>) -> DispatchResult {
        let current_collateral = ext::collateral::for_account::<T>(&self.data.id);

        let raw_current_collateral = Module::<T>::dot_to_u128(current_collateral)?;
        let raw_collateral = Module::<T>::dot_to_u128(collateral)?;
        let raw_new_collateral = raw_current_collateral
            .checked_sub(raw_collateral)
            .unwrap_or(0);

        let new_collateral = Module::<T>::u128_to_dot(raw_new_collateral)?;

        let tokens = self
            .data
            .issued_tokens
            .checked_add(&self.data.to_be_issued_tokens)
            .ok_or(Error::<T>::ArithmeticOverflow)?;
        ensure!(
            !Module::<T>::is_collateral_below_secure_threshold(new_collateral, tokens)?,
            Error::<T>::InsufficientCollateral
        );

        ext::collateral::release_collateral::<T>(&self.data.id, collateral)
    }

    pub fn get_collateral(&self) -> DOT<T> {
        ext::collateral::for_account::<T>(&self.data.id)
    }

    pub fn get_free_collateral(&self) -> Result<DOT<T>, DispatchError> {
        let used_collateral = self.get_used_collateral()?;
        Ok(self
            .get_collateral()
            .checked_sub(&used_collateral)
            .ok_or(Error::<T>::ArithmeticUnderflow)?)
    }

    pub fn get_used_collateral(&self) -> Result<DOT<T>, DispatchError> {
        let issued_tokens = self.data.issued_tokens + self.data.to_be_issued_tokens;
        let issued_tokens_in_dot = ext::oracle::btc_to_dots::<T>(issued_tokens)?;

        let raw_issued_tokens_in_dot = Module::<T>::dot_to_u128(issued_tokens_in_dot)?;

        let secure_threshold = Module::<T>::secure_collateral_threshold();

        let raw_used_collateral = secure_threshold
            .checked_mul_int(raw_issued_tokens_in_dot)
            .ok_or(Error::<T>::ArithmeticOverflow)?;

        let used_collateral = Module::<T>::u128_to_dot(raw_used_collateral)?;

        Ok(used_collateral)
    }

    pub fn issuable_tokens(&self) -> Result<PolkaBTC<T>, DispatchError> {
        let free_collateral = self.get_free_collateral()?;

        let secure_threshold = Module::<T>::secure_collateral_threshold();

        let issuable = Module::<T>::calculate_max_polkabtc_from_collateral_for_threshold(
            free_collateral,
            secure_threshold,
        )?;

        Ok(issuable)
    }

    pub fn increase_to_be_issued(&mut self, tokens: PolkaBTC<T>) -> DispatchResult {
        let issuable_tokens = self.issuable_tokens()?;
        ensure!(issuable_tokens >= tokens, Error::<T>::ExceedingVaultLimit);
        Ok(self.force_increase_to_be_issued(tokens))
    }

    pub fn decrease_to_be_issued(&mut self, tokens: PolkaBTC<T>) -> DispatchResult {
        ensure!(
            self.data.to_be_issued_tokens >= tokens,
            Error::<T>::InsufficientTokensCommitted
        );
        Ok(self.update(|v| v.to_be_issued_tokens -= tokens))
    }

    pub fn issue_tokens(&mut self, tokens: PolkaBTC<T>) -> DispatchResult {
        self.decrease_to_be_issued(tokens)?;
        Ok(self.force_issue_tokens(tokens))
    }

    pub fn increase_to_be_redeemed(&mut self, tokens: PolkaBTC<T>) -> DispatchResult {
        let redeemable = self
            .data
            .issued_tokens
            .checked_sub(&self.data.to_be_redeemed_tokens)
            .ok_or(Error::<T>::ArithmeticUnderflow)?;
        ensure!(
            redeemable >= tokens,
            Error::<T>::InsufficientTokensCommitted
        );
        Ok(self.force_increase_to_be_redeemed(tokens))
    }

    pub fn decrease_to_be_redeemed(&mut self, tokens: PolkaBTC<T>) -> DispatchResult {
        let to_be_redeemed = self.data.to_be_redeemed_tokens;
        ensure!(
            to_be_redeemed >= tokens,
            Error::<T>::InsufficientTokensCommitted
        );
        Ok(self.update(|v| v.to_be_redeemed_tokens -= tokens))
    }

    pub fn decrease_tokens(&mut self, tokens: PolkaBTC<T>) -> DispatchResult {
        self.decrease_to_be_redeemed(tokens)?;
        self.decrease_issued(tokens)
        // Note: slashing of collateral must be called where this function is called (e.g. in Redeem)
    }

    pub fn redeem_tokens(&mut self, tokens: PolkaBTC<T>) -> DispatchResult {
        self.decrease_tokens(tokens)
    }

    pub fn transfer(&mut self, other: &mut RichVault<T>, tokens: PolkaBTC<T>) -> DispatchResult {
        self.decrease_tokens(tokens)?;
        Ok(other.force_issue_tokens(tokens))
    }

    pub fn liquidate<V: UpdatableVault<T>>(
        &mut self,
        liquidation_vault: &mut V,
        status: VaultStatus,
    ) -> DispatchResult {
        let to_slash = Module::<T>::calculate_collateral(
            self.get_collateral(),
            self.data
                .issued_tokens
                .checked_sub(&self.data.to_be_redeemed_tokens)
                .ok_or(Error::<T>::ArithmeticUnderflow)?,
            self.data.issued_tokens,
        )?;

        ext::collateral::slash_collateral::<T>(&self.id(), &liquidation_vault.id(), to_slash)?;

        // Copy all tokens to the liquidation vault
        liquidation_vault.force_issue_tokens(self.data.issued_tokens);
        liquidation_vault.force_increase_to_be_issued(self.data.to_be_issued_tokens);
        liquidation_vault.force_increase_to_be_redeemed(self.data.to_be_redeemed_tokens);

        // Update vault: clear to_be_issued & issued_tokens, but don't touch to_be_redeemed
        self.update(|v| {
            v.to_be_issued_tokens = 0u32.into();
            v.issued_tokens = 0u32.into();
            v.status = status;
        });

        Ok(())
    }

    pub fn ensure_not_banned(&self, height: T::BlockNumber) -> DispatchResult {
        let is_banned = match self.data.banned_until {
            None => false,
            Some(until) => height <= until,
        };

        if is_banned {
            Err(Error::<T>::VaultBanned.into())
        } else {
            Ok(())
        }
    }

    pub fn ban_until(&mut self, height: T::BlockNumber) {
        self.update(|v| v.banned_until = Some(height));
    }

    fn new_deposit_public_key(&self, secure_id: H256) -> Result<BtcPublicKey, DispatchError> {
        let vault_public_key = self.data.wallet.public_key.clone();
        let vault_public_key = vault_public_key
            .new_deposit_public_key(secure_id)
            .map_err(|_| Error::<T>::InvalidPublicKey)?;

        Ok(vault_public_key)
    }

    pub fn insert_deposit_address(&mut self, btc_address: BtcAddress) {
        self.update(|v| {
            v.wallet.add_btc_address(btc_address);
        });
    }

    pub fn new_deposit_address(&mut self, secure_id: H256) -> Result<BtcAddress, DispatchError> {
        let public_key = self.new_deposit_public_key(secure_id)?;
        let btc_address = BtcAddress::P2WPKHv0(public_key.to_hash());
        self.insert_deposit_address(btc_address);
        Ok(btc_address)
    }

    pub fn update_public_key(&mut self, public_key: BtcPublicKey) {
        self.update(|v| {
            v.wallet.public_key = public_key.clone();
        });
    }

    fn update<F, R>(&mut self, func: F) -> ()
    where
        F: Fn(&mut DefaultVault<T>) -> R,
    {
        func(&mut self.data);
        <crate::Vaults<T>>::mutate(&self.data.id, func);
    }
}

impl<T: Config> From<&RichVault<T>> for DefaultVault<T> {
    fn from(rv: &RichVault<T>) -> DefaultVault<T> {
        rv.data.clone()
    }
}

impl<T: Config> From<DefaultVault<T>> for RichVault<T> {
    fn from(vault: DefaultVault<T>) -> RichVault<T> {
        RichVault { data: vault }
    }
}

pub(crate) struct RichSystemVault<T: Config> {
    pub(crate) data: DefaultSystemVault<T>,
}

impl<T: Config> UpdatableVault<T> for RichSystemVault<T> {
    fn id(&self) -> T::AccountId {
        self.data.id.clone()
    }

    fn issued_tokens(&self) -> PolkaBTC<T> {
        self.data.issued_tokens
    }

    fn force_issue_tokens(&mut self, tokens: PolkaBTC<T>) -> () {
        self.update(|v| v.issued_tokens += tokens)
    }

    fn force_increase_to_be_issued(&mut self, tokens: PolkaBTC<T>) -> () {
        self.update(|v| v.to_be_issued_tokens += tokens);
    }

    fn force_increase_to_be_redeemed(&mut self, tokens: PolkaBTC<T>) -> () {
        self.update(|v| v.to_be_redeemed_tokens += tokens);
    }

    fn force_decrease_issued(&mut self, tokens: PolkaBTC<T>) -> () {
        self.update(|v| v.issued_tokens -= tokens);
    }

    fn force_decrease_to_be_issued(&mut self, tokens: PolkaBTC<T>) -> () {
        self.update(|v| v.to_be_issued_tokens -= tokens);
    }

    fn force_decrease_to_be_redeemed(&mut self, tokens: PolkaBTC<T>) -> () {
        self.update(|v| v.to_be_redeemed_tokens -= tokens);
    }
}

#[cfg_attr(test, mockable)]
impl<T: Config> RichSystemVault<T> {
    fn update<F>(&mut self, func: F) -> ()
    where
        F: Fn(&mut DefaultSystemVault<T>) -> (),
    {
        func(&mut self.data);
        <crate::LiquidationVault<T>>::set(self.data.clone());
    }
}

impl<T: Config> From<&RichSystemVault<T>> for DefaultSystemVault<T> {
    fn from(rv: &RichSystemVault<T>) -> DefaultSystemVault<T> {
        rv.data.clone()
    }
}

impl<T: Config> From<DefaultSystemVault<T>> for RichSystemVault<T> {
    fn from(vault: DefaultSystemVault<T>) -> RichSystemVault<T> {
        RichSystemVault { data: vault }
    }
}
