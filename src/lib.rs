#![cfg_attr(not(feature = "std"), no_std)]

use concordium_std::*;
type ContractTokenId = TokenIdU8; // Define ContractTokenId as an alias for TokenIdU8
type ContractTokenAmount = TokenAmountU64; // Define ContractTokenAmount as an alias for TokenAmountU64
use concordium_cis2::{AdditionalData, Cis2Client, Cis2ClientError, OnReceivingCis2Params, Receiver, TokenAmountU64, TokenIdU8, Transfer, TransferParams};


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
    token_contract: ContractAddress, // CIS-2 token contract address
    token_id: TokenIdU8,               // CIS-2 token ID
    token_amount: TokenAmountU64,               // Amount of tokens
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
    pub token_contract: ContractAddress, // CIS-2 token contract address
    pub token_id: TokenIdU8,              // CIS-2 token ID
    pub token_amount: TokenAmountU64,              // Amount of tokens
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
) -> Result<(), BidError> {
    let parameter: NewAuctionParameter = ctx.parameter_cursor().get().map_err(|_| BidError::ParameterParsingError)?;

    let owner = match ctx.sender() {
        Address::Account(account_address) => account_address,
        _ => return Err(BidError::OnlyAccount), // Only accounts can create auctions
    };

    // Transfer CIS-2 tokens from the auction creator to the contract
     let transfer = Transfer {
        token_id: parameter.token_id,
        amount: parameter.token_amount,
        from: Address::Account(owner),
        to: Receiver::from_contract(ctx.self_address(), OwnedEntrypointName::new_unchecked("onReceivingCIS2".to_string())),
        data: AdditionalData::empty(),
    };

    let client = Cis2Client::new(ContractAddress::new(parameter.token_contract.index, parameter.token_contract.subindex));
    let result: Result<bool, Cis2ClientError<()>> = client.transfer(host, transfer);
    // if let Err(err) = &result {
    //     logger.log(&format!("Transfer failed: {:?}", err)).map_err(|_| BidError::TransferFailed)?;
    //     return Err(BidError::TransferFailed);
    // }

    let auction = Auction {
        auction_state: AuctionState::NotSoldYet,
        highest_bidder: None,
        initial_price: parameter.initial_price,
        highest_bid: Amount::zero(),
        item: parameter.item,
        end: parameter.end,
        owner,
        token_contract: parameter.token_contract,
        token_id: parameter.token_id,
        token_amount: parameter.token_amount,
    };

    // Add the new auction to the array
    let state = host.state_mut();
    state.auctions.push(auction);

    // Return the ID of the newly created auction
    let id = (state.auctions.len() - 1) as u32;
    logger.log(&AuctionEvent::Register(AuctionEventData { auction_id: id })).map_err(|_| BidError::TransferFailed)?;
    Ok(())
}

/// Function to handle receiving CIS-2 tokens.
#[receive(contract = "auction", name = "onReceivingCIS2", mutable)]
pub fn on_receiving_cis2(
    ctx: &impl HasReceiveContext,
    host: &mut Host<State>
) -> Result<(), ()> {
    // // Get information about received tokens
    // let params: OnReceivingCis2Params<ContractTokenId, ContractTokenAmount> = ctx.parameter_cursor().get().map_err(|_| ())?;

    // // Get the token contract that sent the tokens
    // let token_contract = match ctx.sender() {
    //     Address::Contract(contract) => contract,
    //     _ => return Ok(()), // Non-contract senders are ignored
    // };

    // // Find the auction that matches the token contract and token ID
    // let state = host.state_mut();
    // if let Some(auction) = state.auctions.iter_mut().find(|auction| {
    //     auction.token_contract == token_contract && auction.token_id == params.token_id
    // }) {
    //     // Assign the received tokens to the auction
    //     auction.token_amount = params.amount;
    // } else {
    //     // If no matching auction is found, return an error
    //     return Err(());
    // }

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
#[receive(contract = "auction", name = "finalize", parameter = "BidParameter", enable_logger, mutable, error = "BidError")]
pub fn auction_finalize(ctx: &impl HasReceiveContext, host: &mut Host<State>, logger: &mut impl HasLogger,) -> Result<(), BidError> {
    let parameter: BidParameter = ctx.parameter_cursor().get().map_err(|_| BidError::ParameterParsingError)?;

    let commission_recipient = host.state().commission_recipient;
    let auction = host.state().auctions.get(parameter.auction_id as usize).ok_or(BidError::AuctionNotFound)?.clone();
    // let auction = {
    //     let auctions = &mut host.state_mut().auctions;
    //     auctions.get_mut(parameter.auction_id as usize).ok_or(BidError::AuctionNotFound)?
    // };

    ensure_eq!(auction.auction_state, AuctionState::NotSoldYet, BidError::AuctionAlreadyFinalized);

    let slot_time = ctx.metadata().slot_time();
    ensure!(slot_time > auction.end, BidError::AuctionStillActive);

    if let Some(winning_bidder) = auction.highest_bidder {
        let commission = auction.highest_bid.micro_ccd / 10;
        let commission_amount = Amount::from_micro_ccd(commission);
        let owner_amount = auction.highest_bid - commission_amount;

        // auction.auction_state = AuctionState::Sold(winning_bidder);

        // Transfer CIS-2 tokens to the highest bidder
        let transfer: Transfer<TokenIdU8, TokenAmountU64> = Transfer {
            token_id: auction.token_id,
            amount: auction.token_amount.into(),
            from: Address::Contract(ctx.self_address()),
            to: Receiver::from_account(winning_bidder),
            data: AdditionalData::empty(),
        };
        let client = Cis2Client::new(ContractAddress::new(auction.token_contract.index, auction.token_contract.subindex));
        let result: Result<bool, Cis2ClientError<()>> = client.transfer(host, transfer);

        logger.log(&format!("{:?}", result)).map_err(|_| BidError::TransferFailed)?;

        host.invoke_transfer(&commission_recipient, commission_amount).map_err(|_| BidError::TransferFailed)?;
        host.invoke_transfer(&auction.owner, owner_amount).map_err(|_| BidError::TransferFailed)?;
    } else {
        // Return CIS-2 tokens to the auction creator
        let transfer: Transfer<TokenIdU8, TokenAmountU64> = Transfer {
            token_id: auction.token_id,
            amount: auction.token_amount.into(),
            from: Address::Contract(ctx.self_address()),
            to: Receiver::from_account(auction.owner),
            data: AdditionalData::empty(),
        };

        let client = Cis2Client::new(ContractAddress::new(auction.token_contract.index, auction.token_contract.subindex));
        let result: Result<bool, Cis2ClientError<()>> = client.transfer(host, transfer);

        logger.log(&format!("{:?}", result)).map_err(|_| BidError::TransferFailed)?;
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
