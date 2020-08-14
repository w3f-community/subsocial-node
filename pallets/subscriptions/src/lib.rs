#![cfg_attr(not(feature = "std"), no_std)]

use codec::{Encode, Decode};
use sp_std::prelude::*;
use sp_runtime::RuntimeDebug;

use frame_support::{
	decl_module, decl_storage, decl_event, decl_error, ensure,
	dispatch::DispatchResult,
	traits::{Get, Currency, ExistenceRequirement}
};
use frame_system::{self as system, ensure_signed};

use pallet_permissions::SpacePermission;
use pallet_spaces::{Module as Spaces, Space};
use pallet_utils::{Module as Utils, SpaceId, Content, WhoAndWhen};

/*#[cfg(test)]
mod mock;

#[cfg(test)]
mod tests;*/

pub mod functions;

pub type SubscriptionPlanId = u64;
pub type SubscriptionId = u64;

#[derive(Encode, Decode, Clone, Eq, PartialEq, RuntimeDebug)]
pub enum SubscriptionPeriod<BlockNumber> {
	Daily,
	Weekly,
	Quarterly,
	Yearly,
	Custom(BlockNumber), // Currently not supported
}

#[derive(Encode, Decode, Clone, Eq, PartialEq, RuntimeDebug)]
pub struct SubscriptionPlan<T: Trait> {
	pub id: SubscriptionPlanId,
	pub created: WhoAndWhen<T>,
	pub updated: Option<WhoAndWhen<T>>,
	pub space_id: SpaceId, // Describes what space is this plan related to
	pub wallet: Option<T::AccountId>,
	pub price: BalanceOf<T>,
	pub period: SubscriptionPeriod<T::BlockNumber>,
	pub content: Content,
	pub is_active: bool,
}

#[derive(Encode, Decode, Clone, Eq, PartialEq, RuntimeDebug)]
pub struct Subscription<T: Trait> {
	pub id: SubscriptionId,
	pub created: WhoAndWhen<T>,
	pub updated: Option<WhoAndWhen<T>>,
	pub wallet: Option<T::AccountId>,
	pub plan_id: SubscriptionPlanId,
	pub is_active: bool,
}

type BalanceOf<T> = <<T as pallet_utils::Trait>::Currency as Currency<<T as system::Trait>::AccountId>>::Balance;

/// The pallet's configuration trait.
pub trait Trait:
	system::Trait
	+ pallet_utils::Trait
	+ pallet_spaces::Trait
{
	/// The overarching event type.
	type Event: From<Event<Self>> + Into<<Self as system::Trait>::Event>;
}

decl_storage! {
	trait Store for Module<T: Trait> as SubscriptionsModule {
		// Plans:

		pub NextPlanId get(fn next_plan_id): SubscriptionPlanId = 1;

		pub PlanById get(fn plan_by_id):
			map hasher(twox_64_concat) SubscriptionPlanId => Option<SubscriptionPlan<T>>;

		pub PlanIdsBySpace get(fn plan_ids_by_space):
			map hasher(twox_64_concat) SpaceId => Vec<SubscriptionPlanId>;

		// Subscriptions:

		pub NextSubscriptionId get(fn next_subscription_id): SubscriptionId = 1;

		pub SubscriptionById get(fn subscription_by_id):
			map hasher(twox_64_concat) SubscriptionId => Option<Subscription<T>>;

		pub SubscriptionIdsByPatron get(fn subscription_ids_by_patron):
			map hasher(blake2_128_concat) T::AccountId => Vec<SubscriptionId>;

		pub SubscriptionIdsBySpace get(fn subscription_ids_by_space):
			map hasher(twox_64_concat) SpaceId => Vec<SubscriptionId>;

		// todo: this should be used by Scheduler to transfer funds from subscribers' wallets to creator's (space) wallet.
		pub SubscriptionIdsByPeriod get(fn subscription_ids_by_period):
			map hasher(twox_64_concat) SubscriptionPeriod<T::BlockNumber> => Vec<SubscriptionId>;

		// Wallets

		// Where to transfer balance withdrawn from subscribers
		pub RecipientWallet get(fn recipient_wallet):
			map hasher(twox_64_concat) SpaceId => Option<T::AccountId>;

		// From where to withdraw subscribers donation
		pub PatronWallet get(fn patron_wallet):
			map hasher(twox_64_concat) T::AccountId => Option<T::AccountId>;
	}
}

// The pallet's events
decl_event!(
	pub enum Event<T> where
		AccountId = <T as system::Trait>::AccountId
	{
		SubscriptionPlanCreated(AccountId, SubscriptionPlanId),
		// todo: complete event list for this pallet once dispatches are implemented
	}
);

decl_error! {
	pub enum Error for Module<T: Trait> {
		AlreadySubscribed,
		NoPermissionToUpdateSubscriptionPlan,
		NotSubscriber,
		NothingToUpdate,
		PriceLowerExistencialDeposit,
		RecipientNotFound,
		SubscriptionNotFound,
		SubscriptionPlanNotFound,
	}
}

decl_module! {
	/// The module declaration.
	pub struct Module<T: Trait> for enum Call where origin: T::Origin {
		// Initializing errors
		type Error = Error<T>;

		// Initializing events
		fn deposit_event() = default;

		#[weight = T::DbWeight::get().reads_writes(3, 3) + 25_000]
		pub fn create_plan(
			origin,
			space_id: SpaceId,
			custom_wallet: Option<T::AccountId>,
			price: BalanceOf<T>,
			period: SubscriptionPeriod<T::BlockNumber>,
			content: Content
		) -> DispatchResult {
			let sender = ensure_signed(origin)?;

			Utils::<T>::is_valid_content(content.clone())?;

			ensure!(
				price >= <T as pallet_utils::Trait>::Currency::minimum_balance(),
				Error::<T>::PriceLowerExistencialDeposit
			);

			let space = Spaces::<T>::require_space(space_id)?;
			space.ensure_space_owner(sender.clone())?;

			// todo:
			// 	- use permission to manage (here: create) subscription plans

			let plan_id = Self::next_plan_id();
			let subscription_plan = SubscriptionPlan::<T>::new(
				plan_id,
				sender,
				space_id,
				custom_wallet,
				price,
				period,
				content
			);

			PlanById::<T>::insert(plan_id, subscription_plan);
			PlanIdsBySpace::mutate(space_id, |ids| ids.push(plan_id));
			NextPlanId::mutate(|x| { *x += 1 });

			Ok(())
		}

		#[weight = T::DbWeight::get().reads_writes(2, 1) + 10_000]
		pub fn update_plan(origin, plan_id: SubscriptionPlanId, new_wallet: Option<T::AccountId>) -> DispatchResult {
			let sender = ensure_signed(origin)?;

			let mut plan = Self::require_plan(plan_id)?;

			let space = Spaces::<T>::require_space(plan.space_id)?;
			Self::ensure_subscriptions_manager(sender, &space)?;

			ensure!(new_wallet != plan.wallet, Error::<T>::NothingToUpdate);
			plan.wallet = new_wallet;
			// todo: change updated field
			PlanById::<T>::insert(plan_id, plan);

			Ok(())
		}

		// todo: split to `set_space_wallet` and `remove_space_wallet`
		#[weight = T::DbWeight::get().reads_writes(1, 1) + 10_000]
		pub fn update_space_default_wallet(
			origin,
			space_id: SpaceId,
			custom_wallet: Option<T::AccountId>
		) -> DispatchResult {
			let sender = ensure_signed(origin)?;

			let space = Spaces::<T>::require_space(space_id)?;
			space.ensure_space_owner(sender)?;

			if let Some(wallet) = custom_wallet {
				RecipientWallet::<T>::insert(space.id, wallet);
			} else {
				RecipientWallet::<T>::remove(space.id);
			}

			Ok(())
		}

		#[weight = 10_000]
		pub fn delete_plan(origin, plan_id: SubscriptionPlanId) -> DispatchResult {
			let _ = ensure_signed(origin)?;
			Ok(())
		}

		#[weight = T::DbWeight::get().reads_writes(5, 1) + 50_000]
		pub fn subscribe(
			origin,
			plan_id: SubscriptionPlanId,
			custom_wallet: Option<T::AccountId>
		) -> DispatchResult {
			let sender = ensure_signed(origin)?;

			let plan = Self::require_plan(plan_id)?;
			let subscriptions = Self::subscription_ids_by_patron(&sender);

			let is_already_subscribed = subscriptions.iter().any(|subscription_id| {
				if let Ok(subscription) = Self::require_subscription(*subscription_id) {
					return subscription.plan_id == plan_id;
				}
				false
			});
			ensure!(is_already_subscribed, Error::<T>::AlreadySubscribed);

			let subscription_id = Self::next_subscription_id();
			let subscription = Subscription::<T>::new(
				subscription_id,
				sender.clone(),
				custom_wallet,
				plan_id
			);

			let recipient = plan.wallet.clone()
				.or_else(|| Self::recipient_wallet(plan.space_id))
				.or_else(|| {
					Spaces::<T>::require_space(plan.space_id).map(|space| space.owner).ok()
				});

			ensure!(recipient.is_some(), Error::<T>::RecipientNotFound);
			<T as pallet_utils::Trait>::Currency::transfer(
				&sender,
				&recipient.unwrap(),
				plan.price,
				ExistenceRequirement::KeepAlive
			)?;

			// todo: schedule recurrent payment

			SubscriptionById::<T>::insert(subscription_id, subscription);

			Ok(())
		}

		#[weight = T::DbWeight::get().reads_writes(1, 1) + 10_000]
		pub fn update_subscribtion(
			origin,
			subscription_id: SubscriptionId,
			new_wallet: Option<T::AccountId>
		) -> DispatchResult {
			let sender = ensure_signed(origin)?;

			let mut subscription = Self::require_subscription(subscription_id)?;
			subscription.ensure_subscriber(&sender)?;

			ensure!(new_wallet != subscription.wallet, Error::<T>::NothingToUpdate);

			// todo: change updated field
			subscription.wallet = new_wallet;
			SubscriptionById::<T>::insert(subscription_id, subscription);

			Ok(())
		}

		// todo: split to `set_subscription_wallet` and `remove_subscription_wallet`
		#[weight = T::DbWeight::get().reads_writes(0, 1) + 10_000]
		pub fn update_subscriptions_default_wallet(
			origin,
			custom_wallet: Option<T::AccountId>
		) -> DispatchResult {
			let sender = ensure_signed(origin)?;

			if let Some(wallet) = custom_wallet {
				PatronWallet::<T>::insert(sender, wallet);
			} else {
				PatronWallet::<T>::remove(sender);
			}

			Ok(())
		}

		#[weight = 10_000]
		pub fn unsubscribe(origin, plan_id: SubscriptionPlanId) -> DispatchResult {
			// todo(i): maybe we need here subscription_id, not plan_id?
			let _ = ensure_signed(origin)?;
			Ok(())
		}
	}
}