#![cfg_attr(not(feature = "std"), no_std)]
#![cfg_attr(not(feature = "std"), feature(core_intrinsics))]
#![cfg_attr(not(feature = "std"), feature(alloc_error_handler))]
#![cfg_attr(feature = "log", feature(panic_info_message))]

#[cfg(not(feature = "std"))]
extern crate alloc;
#[cfg(not(feature = "std"))]
extern crate core;

pub mod meta_parsing;
pub mod parameters;
mod precompiles;
pub mod prelude;
mod storage;
mod transaction;
pub mod types;

#[cfg(feature = "contract")]
mod connector;
#[cfg(feature = "contract")]
mod deposit_event;
#[cfg(feature = "contract")]
mod engine;
#[cfg(feature = "contract")]
mod fungible_token;
#[cfg(feature = "contract")]
mod log_entry;
#[cfg(feature = "contract")]
mod prover;
#[cfg(feature = "contract")]
mod sdk;

#[cfg(feature = "contract")]
mod contract {
    use borsh::BorshDeserialize;
    use evm::{ExitError, ExitFatal, ExitReason};

    use crate::connector::EthConnectorContract;
    use crate::engine::{Engine, EngineState};
    #[cfg(feature = "evm_bully")]
    use crate::parameters::{BeginBlockArgs, BeginChainArgs};
    use crate::parameters::{
        DeployEvmTokenCallArgs, FunctionCallArgs, GetStorageAtArgs, NewCallArgs, ViewCallArgs,
    };
    use crate::prelude::{vec, Address, H256, U256};
    use crate::sdk;
    use crate::types::{near_account_to_evm_address, u256_to_arr};

    #[global_allocator]
    static ALLOC: wee_alloc::WeeAlloc = wee_alloc::WeeAlloc::INIT;

    const CODE_KEY: &[u8; 5] = b"\0CODE";
    const CODE_STAGE_KEY: &[u8; 11] = b"\0CODE_STAGE";

    #[cfg(target_arch = "wasm32")]
    #[panic_handler]
    #[cfg_attr(not(feature = "log"), allow(unused_variables))]
    #[no_mangle]
    pub unsafe fn on_panic(info: &::core::panic::PanicInfo) -> ! {
        #[cfg(feature = "log")]
        {
            use alloc::string::ToString;
            if let Some(msg) = info.message() {
                let msg = if let Some(log) = info.location() {
                    [msg.to_string(), " [".into(), log.to_string(), "]".into()].join("")
                } else {
                    msg.to_string()
                };
                sdk::log(msg);
            } else if let Some(log) = info.location() {
                sdk::log(log.to_string());
            }
        }

        ::core::intrinsics::abort();
    }

    #[cfg(target_arch = "wasm32")]
    #[alloc_error_handler]
    #[no_mangle]
    pub unsafe fn on_alloc_error(_: core::alloc::Layout) -> ! {
        ::core::intrinsics::abort();
    }

    ///
    /// ADMINISTRATIVE METHODS
    ///

    /// Sets the configuration for the Engine.
    /// Should be called on deployment.
    #[no_mangle]
    pub extern "C" fn new() {
        let state = Engine::get_state();
        if !state.owner_id.is_empty() {
            require_owner_only(state);
        }
        let args = NewCallArgs::try_from_slice(&sdk::read_input()).expect("ERR_ARG_PARSE");
        Engine::set_state(args.into());
    }

    /// Get version of the contract.
    #[no_mangle]
    pub extern "C" fn get_version() {
        let version = match option_env!("NEAR_EVM_VERSION") {
            Some(v) => v.as_bytes(),
            None => include_bytes!("../VERSION"),
        };
        sdk::return_output(version)
    }

    /// Get owner account id for this contract.
    #[no_mangle]
    pub extern "C" fn get_owner() {
        let state = Engine::get_state();
        sdk::return_output(state.owner_id.as_bytes());
    }

    /// Get bridge prover id for this contract.
    #[no_mangle]
    pub extern "C" fn get_bridge_provider() {
        let state = Engine::get_state();
        sdk::return_output(state.bridge_prover_id.as_bytes());
    }

    /// Get chain id for this contract.
    #[no_mangle]
    pub extern "C" fn get_chain_id() {
        sdk::return_output(&Engine::get_state().chain_id)
    }

    #[no_mangle]
    pub extern "C" fn get_upgrade_index() {
        let state = Engine::get_state();
        let index = sdk::read_u64(CODE_STAGE_KEY).expect("ERR_NO_UPGRADE");
        sdk::return_output(&(index + state.upgrade_delay_blocks).to_le_bytes())
    }

    /// Stage new code for deployment.
    #[no_mangle]
    pub extern "C" fn stage_upgrade() {
        let state = Engine::get_state();
        require_owner_only(state);
        sdk::read_input_and_store(CODE_KEY);
        sdk::write_storage(CODE_STAGE_KEY, &sdk::block_index().to_le_bytes());
    }

    /// Deploy staged upgrade.
    #[no_mangle]
    pub extern "C" fn deploy_upgrade() {
        let state = Engine::get_state();
        let index = sdk::read_u64(CODE_STAGE_KEY).unwrap();
        if sdk::block_index() <= index + state.upgrade_delay_blocks {
            sdk::panic_utf8(b"ERR_NOT_ALLOWED:TOO_EARLY");
        }
        sdk::self_deploy(CODE_KEY);
    }

    ///
    /// MUTATIVE METHODS
    ///

    /// Deploy code into the EVM.
    #[no_mangle]
    pub extern "C" fn deploy_code() {
        let input = sdk::read_input();
        let mut engine = Engine::new(predecessor_address());
        let (status, address) = Engine::deploy_code_with_input(&mut engine, &input);
        // TODO: charge for storage
        process_exit_reason(status, &address.0)
    }

    /// Call method on the EVM contract.
    #[no_mangle]
    pub extern "C" fn call() {
        let input = sdk::read_input();
        let args = FunctionCallArgs::try_from_slice(&input).expect("ERR_ARG_PARSE");
        let mut engine = Engine::new(predecessor_address());
        let (status, result) = Engine::call_with_args(&mut engine, args);
        // TODO: charge for storage
        process_exit_reason(status, &result)
    }

    /// Process signed Ethereum transaction.
    /// Must match CHAIN_ID to make sure it's signed for given chain vs replayed from another chain.
    #[no_mangle]
    pub extern "C" fn raw_call() {
        use crate::transaction::EthSignedTransaction;
        use rlp::{Decodable, Rlp};

        let input = sdk::read_input();
        let signed_transaction = EthSignedTransaction::decode(&Rlp::new(&input))
            .map_err(|_| ())
            .expect("ERR_INVALID_TX");

        let state = Engine::get_state();

        // Validate the chain ID, if provided inside the signature:
        if let Some(chain_id) = signed_transaction.chain_id() {
            if U256::from(chain_id) != U256::from(state.chain_id) {
                sdk::panic_utf8(b"ERR_INVALID_CHAIN_ID");
            }
        }

        // Retrieve the signer of the transaction:
        let sender = match signed_transaction.sender() {
            Some(sender) => sender,
            None => sdk::panic_utf8(b"ERR_INVALID_ECDSA_SIGNATURE"),
        };

        // Figure out what kind of a transaction this is, and execute it:
        let mut engine = Engine::new_with_state(state, sender);
        let value = signed_transaction.transaction.value;
        let data = signed_transaction.transaction.data;
        if let Some(receiver) = signed_transaction.transaction.to {
            let (status, result) = if data.is_empty() {
                // Execute a balance transfer:
                (
                    Engine::transfer(&mut engine, &sender, &receiver, &value),
                    vec![],
                )
            } else {
                // Execute a contract call:
                Engine::call(&mut engine, sender, receiver, value, data)
                // TODO: charge for storage
            };
            process_exit_reason(status, &result)
        } else {
            // Execute a contract deployment:
            let (status, result) = Engine::deploy_code(&mut engine, sender, value, &data);
            // TODO: charge for storage
            process_exit_reason(status, &result.0)
        }
    }

    #[no_mangle]
    pub extern "C" fn meta_call() {
        let input = sdk::read_input();
        let state = Engine::get_state();
        let domain_separator = crate::meta_parsing::near_erc712_domain(U256::from(state.chain_id));
        let meta_call_args = match crate::meta_parsing::parse_meta_call(
            &domain_separator,
            &sdk::current_account_id(),
            input,
        ) {
            Ok(args) => args,
            Err(_error_kind) => {
                sdk::panic_utf8(b"ERR_META_TX_PARSE");
            }
        };
        let mut engine = Engine::new_with_state(state, meta_call_args.sender);
        let (status, result) = engine.call(
            meta_call_args.sender,
            meta_call_args.contract_address,
            meta_call_args.value,
            meta_call_args.input,
        );
        process_exit_reason(status, &result);
    }

    ///
    /// NONMUTATIVE METHODS
    ///

    #[no_mangle]
    pub extern "C" fn view() {
        let input = sdk::read_input();
        let args = ViewCallArgs::try_from_slice(&input).expect("ERR_ARG_PARSE");
        let engine = Engine::new(Address::from_slice(&args.sender));
        let (status, result) = Engine::view_with_args(&engine, args);
        process_exit_reason(status, &result)
    }

    #[no_mangle]
    pub extern "C" fn get_code() {
        let address = sdk::read_input_arr20();
        let code = Engine::get_code(&Address(address));
        sdk::return_output(&code)
    }

    #[no_mangle]
    pub extern "C" fn get_balance() {
        let address = sdk::read_input_arr20();
        let balance = Engine::get_balance(&Address(address));
        sdk::return_output(&u256_to_arr(&balance))
    }

    #[no_mangle]
    pub extern "C" fn get_nonce() {
        let address = sdk::read_input_arr20();
        let nonce = Engine::get_nonce(&Address(address));
        sdk::return_output(&u256_to_arr(&nonce))
    }

    #[no_mangle]
    pub extern "C" fn get_storage_at() {
        let input = sdk::read_input();
        let args = GetStorageAtArgs::try_from_slice(&input).expect("ERR_ARG_PARSE");
        let value = Engine::get_storage(&Address(args.address), &H256(args.key));
        sdk::return_output(&value.0)
    }

    ///
    /// BENCHMARKING METHODS
    ///

    #[cfg(feature = "evm_bully")]
    #[no_mangle]
    pub extern "C" fn begin_chain() {
        let mut state = Engine::get_state();
        require_owner_only(state);
        let input = sdk::read_input();
        let args = BeginChainArgs::try_from_slice(&input).expect("ERR_ARG_PARSE");
        state.chain_id = args.chain_id;
        Engine::set_state(state);
        // TODO: https://github.com/aurora-is-near/aurora-engine/issues/1
    }

    #[cfg(feature = "evm_bully")]
    #[no_mangle]
    pub extern "C" fn begin_block() {
        let state = Engine::get_state();
        require_owner_only(state);
        let input = sdk::read_input();
        let _args = BeginBlockArgs::try_from_slice(&input).expect("ERR_ARG_PARSE");
        // TODO: https://github.com/aurora-is-near/aurora-engine/issues/2
    }

    #[no_mangle]
    pub extern "C" fn new_eth_connector() {
        EthConnectorContract::init_contract()
    }

    #[no_mangle]
    pub extern "C" fn withdraw() {
        EthConnectorContract::new().withdraw_near()
    }

    #[no_mangle]
    pub extern "C" fn deposit() {
        EthConnectorContract::new().deposit()
    }

    #[no_mangle]
    pub extern "C" fn finish_deposit_near() {
        EthConnectorContract::new().finish_deposit_near();
    }

    #[no_mangle]
    pub extern "C" fn finish_deposit_eth() {
        EthConnectorContract::new().finish_deposit_eth();
    }

    #[no_mangle]
    pub extern "C" fn ft_total_supply() {
        EthConnectorContract::new().ft_total_supply();
    }

    #[no_mangle]
    pub extern "C" fn ft_total_supply_near() {
        EthConnectorContract::new().ft_total_supply_near();
    }

    #[no_mangle]
    pub extern "C" fn ft_total_supply_eth() {
        EthConnectorContract::new().ft_total_supply_eth();
    }

    #[no_mangle]
    pub extern "C" fn ft_balance_of() {
        EthConnectorContract::new().ft_balance_of();
    }

    #[no_mangle]
    pub extern "C" fn ft_balance_of_eth() {
        EthConnectorContract::new().ft_balance_of_eth();
    }

    #[no_mangle]
    pub extern "C" fn ft_transfer() {
        EthConnectorContract::new().ft_transfer();
    }

    #[no_mangle]
    pub extern "C" fn ft_resolve_transfer() {
        EthConnectorContract::new().ft_resolve_transfer();
    }

    #[no_mangle]
    pub extern "C" fn ft_transfer_call() {
        EthConnectorContract::new().ft_transfer_call();
    }

    #[no_mangle]
    pub extern "C" fn storage_deposit() {
        EthConnectorContract::new().storage_deposit()
    }

    #[no_mangle]
    pub extern "C" fn storage_withdraw() {
        EthConnectorContract::new().storage_withdraw()
    }

    #[no_mangle]
    pub extern "C" fn storage_balance_of() {
        EthConnectorContract::new().storage_balance_of()
    }

    #[no_mangle]
    pub extern "C" fn register_relayer() {
        EthConnectorContract::new().register_relayer()
    }

    /// Deploy ERC20 contract and save to storage ERC20 account to NEAR account alias
    #[no_mangle]
    pub extern "C" fn deploy_evm_token() {
        let args =
            DeployEvmTokenCallArgs::try_from_slice(&sdk::read_input()).expect("ERR_ARG_PARSE");
        let mut engine = Engine::new(predecessor_address());
        let (status, address) = Engine::deploy_code_with_input(&mut engine, &args.erc20_contract);
        if let ExitReason::Succeed(_) = status {
            EthConnectorContract::new().save_evm_token_address(&args.near_account_id, address.0);
        }
        process_exit_reason(status, &address.0)
    }

    #[no_mangle]
    pub extern "C" fn ft_on_transfer() {
        EthConnectorContract::new().ft_on_transfer()
    }

    #[cfg(feature = "integration-test")]
    #[no_mangle]
    pub extern "C" fn verify_log_entry() {
        use borsh::BorshSerialize;
        #[cfg(feature = "log")]
        sdk::log("Call from verify_log_entry".into());
        let data = true.try_to_vec().unwrap();
        sdk::return_output(&data[..]);
    }

    ///
    /// Utility methods.
    ///

    fn require_owner_only(state: EngineState) {
        if state.owner_id.as_bytes() != sdk::predecessor_account_id() {
            sdk::panic_utf8(b"ERR_NOT_ALLOWED");
        }
    }

    fn predecessor_address() -> Address {
        near_account_to_evm_address(&sdk::predecessor_account_id())
    }

    fn process_exit_reason(status: ExitReason, result: &[u8]) {
        match status {
            ExitReason::Succeed(_) => sdk::return_output(result),
            ExitReason::Revert(_) => sdk::panic_hex(&result),
            ExitReason::Error(error) => sdk::panic_utf8(error.to_str().as_bytes()),
            ExitReason::Fatal(error) => sdk::panic_utf8(error.to_str().as_bytes()),
        }
    }

    trait ToStr {
        fn to_str(&self) -> &'static str;
    }

    impl ToStr for ExitError {
        fn to_str(&self) -> &'static str {
            match self {
                ExitError::StackUnderflow => "StackUnderflow",
                ExitError::StackOverflow => "StackOverflow",
                ExitError::InvalidJump => "InvalidJump",
                ExitError::InvalidRange => "InvalidRange",
                ExitError::DesignatedInvalid => "DesignatedInvalid",
                ExitError::CallTooDeep => "CallTooDeep",
                ExitError::CreateCollision => "CreateCollision",
                ExitError::CreateContractLimit => "CreateContractLimit",
                ExitError::OutOfOffset => "OutOfOffset",
                ExitError::OutOfGas => "OutOfGas",
                ExitError::OutOfFund => "OutOfFund",
                ExitError::PCUnderflow => "PCUnderflow",
                ExitError::CreateEmpty => "CreateEmpty",
                ExitError::Other(_) => "Other",
            }
        }
    }

    impl ToStr for ExitFatal {
        fn to_str(&self) -> &'static str {
            match self {
                ExitFatal::NotSupported => "NotSupported",
                ExitFatal::UnhandledInterrupt => "UnhandledInterrupt",
                ExitFatal::CallErrorAsFatal(_) => "CallErrorAsFatal",
                ExitFatal::Other(_) => "Other",
            }
        }
    }
}
