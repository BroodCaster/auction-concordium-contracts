#![cfg_attr(not(feature = "std"), no_std)]

use concordium_std::*;

/// The state of an auction.
#[derive(Debug, Serialize, SchemaType, Eq, PartialEq, PartialOrd, Clone)]
pub enum AuctionState {
    NotSoldYet,
    Sold(AccountAddress),
}

#[derive(Debug, PartialEq, Eq, Serialize, SchemaType)]
pub struct AuctionEventData {
    pub auction_id:       u32,
}

#[derive(Debug, PartialEq, Serialize, Eq)]
pub enum AuctionEvent {
    Register(AuctionEventData),
}

/// Auction struct representing a single auction.
#[derive(Debug, Serialize, SchemaType, Clone)]
pub struct Auction {
    auction_state: AuctionState,
    highest_bidder: Option<AccountAddress>,
    initial_price: u64,
    highest_bid: Amount,
    item: String,
    end: Timestamp,
    owner: AccountAddress,
}

/// The state of the smart contract.
#[derive(Debug, Serialize, SchemaType, Clone)]
pub struct State {
    auctions: Vec<Auction>,  // Array of auctions
    commission_recipient: AccountAddress,
}

/// Type of the parameter to create a new auction.
#[derive(Serialize, SchemaType)]
pub struct NewAuctionParameter {
    pub item: String,
    pub end: Timestamp,
    pub initial_price: u64,
}

/// Type of the parameter to place a bid.
#[derive(Serialize, SchemaType)]
pub struct BidParameter {
    pub auction_id: u32,  // ID of the auction to bid on
}

/// Errors for bidding function.
#[derive(Debug, PartialEq, Eq, Clone, Reject, Serialize, SchemaType)]
pub enum BidError {
    OnlyAccount,
    BidBelowCurrentBid,
    BidBelowMinimumRaise,
    BidTooLate,
    AuctionAlreadyFinalized,
    AuctionNotFound,
    ParameterParsingError,
    AuctionStillActive,
    TransferFailed,
    OnlyNotOwner
}

/// `create_auction` function to add a new auction to the array.
#[receive(contract = "auction", name = "create_auction", parameter = "NewAuctionParameter", enable_logger, mutable)]
pub fn create_auction(
    ctx: &impl HasReceiveContext,
    host: &mut Host<State>,
    logger: &mut impl HasLogger,
) -> Result<(), ()> {
    let parameter: NewAuctionParameter = ctx.parameter_cursor().get().map_err(|_| ())?;

    let owner = match ctx.sender() {
        Address::Account(account_address) => account_address,
        _ => return Err(()), // Only accounts can create auctions
    };

    let auction = Auction {
        auction_state: AuctionState::NotSoldYet,
        highest_bidder: None,
        initial_price: parameter.initial_price,
        highest_bid: Amount::zero(),
        item: parameter.item,
        end: parameter.end,
        owner, // Set the owner of the auction
    };

    // Add the new auction to the array
    let state = host.state_mut();
    state.auctions.push(auction);

    // Return the ID of the newly created auction
    let id = (state.auctions.len() - 1) as u32;
    // logger.log(&AuctionEvent::Register(AuctionEventData {
    //     auction_id: auction_id,
    // }))?;
    logger.log(&AuctionEvent::Register(AuctionEventData{ auction_id: id })).map_err(|_| ())?;
    Ok(())
}

/// `bid` function to place a bid on a specific auction.
#[receive(contract = "auction", name = "bid", parameter = "BidParameter", payable, mutable, error = "BidError")]
pub fn auction_bid(
    ctx: &impl HasReceiveContext,
    host: &mut Host<State>,  // Use &mut Host<State> for state-modifying functions
    amount: Amount,
) -> Result<(), BidError> {
    let parameter: BidParameter = ctx.parameter_cursor().get().map_err(|_| BidError::ParameterParsingError)?;

    // Get mutable access to the auction, and ensure it exists
    let auction = {
        let auctions = &mut host.state_mut().auctions;
        auctions.get_mut(parameter.auction_id as usize).ok_or(BidError::AuctionNotFound)?
    };

    // Ensure the auction has not been finalized yet
    ensure_eq!(auction.auction_state, AuctionState::NotSoldYet, BidError::AuctionAlreadyFinalized);

    let slot_time = ctx.metadata().slot_time();
    // Ensure the auction has not ended yet
    ensure!(slot_time <= auction.end, BidError::BidTooLate);

    // Ensure that only accounts can place a bid
    let sender_address = match ctx.sender() {
        Address::Contract(_) => bail!(BidError::OnlyAccount),
        Address::Account(account_address) => account_address,
    };

    ensure!(auction.owner != sender_address, BidError::OnlyNotOwner);

    // Check if the current highest bid is zero
    if auction.highest_bid == Amount::zero() {
        // Ensure the bid is greater than the initial price
        ensure!(amount.micro_ccd > auction.initial_price, BidError::BidBelowCurrentBid);
    } else {
        // Ensure that the new bid exceeds the current highest bid
        ensure!(amount > auction.highest_bid, BidError::BidBelowCurrentBid);
    }

    // Extract necessary fields from `auction` before releasing mutable borrow
    let previous_highest_bid = auction.highest_bid;
    let prev_highest_bidder = auction.highest_bidder.take();

    // Update auction with new highest bid and highest bidder
    auction.highest_bid = amount;
    auction.highest_bidder = Some(sender_address);

    // Refund previous highest bidder, if any
    if let Some(prev_bidder) = prev_highest_bidder {
        // Refund the previous highest bid
        host.invoke_transfer(&prev_bidder, previous_highest_bid).unwrap_abort();
    }

    Ok(())
}

/// View function to return the array of auctions.
#[receive(contract = "auction", name = "view_auctions", return_value = "Vec<Auction>")]
pub fn view_auctions(_ctx: &impl HasReceiveContext, host: &Host<State>) -> ReceiveResult<Vec<Auction>> {
    Ok(host.state().auctions.clone())
}

/// `get_auction` function to fetch a specific auction by its ID as a view function.
#[receive(contract = "auction", name = "get_auction", parameter = "BidParameter", return_value = "Auction")]
pub fn get_auction(
    ctx: &impl HasReceiveContext,
    host: &Host<State>,
) -> ReceiveResult<Auction> {
    let parameter: BidParameter = ctx.parameter_cursor().get().map_err(|_| BidError::ParameterParsingError)?;

    // Get immutable access to the auction, and ensure it exists
    let auction = host
        .state()
        .auctions
        .get(parameter.auction_id as usize)
        .ok_or(BidError::AuctionNotFound)?;

    Ok(auction.clone())
}


/// `finalize` function to finalize a specific auction.
#[receive(contract = "auction", name = "finalize", parameter = "BidParameter", mutable, error = "BidError")]
pub fn auction_finalize(ctx: &impl HasReceiveContext, host: &mut Host<State>) -> Result<(), BidError> {
    let parameter: BidParameter = ctx.parameter_cursor().get().map_err(|_| BidError::ParameterParsingError)?;

    let commission_recipient = host.state().commission_recipient;
    // Get mutable access to the auction, and ensure it exists
    let auction = {
        let auctions = &mut host.state_mut().auctions;
        auctions.get_mut(parameter.auction_id as usize).ok_or(BidError::AuctionNotFound)?
    };

    // Ensure the auction has not been finalized yet
    ensure_eq!(auction.auction_state, AuctionState::NotSoldYet, BidError::AuctionAlreadyFinalized);

    let slot_time = ctx.metadata().slot_time();
    // Ensure the auction has ended already
    ensure!(slot_time > auction.end, BidError::AuctionStillActive);

    if let Some(winning_bidder) = auction.highest_bidder {
        // Calculate the commission (10%)
        let commission = auction.highest_bid.micro_ccd / 10;
        let commission_amount = Amount::from_micro_ccd(commission);
        let owner_amount = auction.highest_bid - commission_amount;

        // Mark the auction as sold
        auction.auction_state = AuctionState::Sold(winning_bidder);
        

        // Send the remaining amount to the auction's owner
        let owner = auction.owner;
        host.invoke_transfer(&commission_recipient, commission_amount).map_err(|_| BidError::TransferFailed)?;
        host.invoke_transfer(&owner, owner_amount).map_err(|_| BidError::TransferFailed)?;
    }

    Ok(())
}

/// Init function to initialize the state with an empty array of auctions.
#[init(contract = "auction")]
pub fn auction_init(_ctx: &InitContext, _state_builder: &mut StateBuilder<ExternStateApi>) -> InitResult<State> {
    let commission_recipient = _ctx.init_origin();

    Ok(State {
        auctions: Vec::new(),  // Start with an empty array of auctions
        commission_recipient,
    })
}
