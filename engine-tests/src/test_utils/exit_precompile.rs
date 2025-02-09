use crate::prelude::{
    parameters::SubmitResult, transaction::LegacyEthTransaction, Address, Wei, U256,
};
use crate::test_utils::{self, solidity, AuroraRunner, Signer};

pub(crate) struct TesterConstructor(pub solidity::ContractConstructor);

const DEPLOY_CONTRACT_GAS: u64 = 1_000_000_000;
pub const DEST_ACCOUNT: &str = "target.aurora";
pub const DEST_ADDRESS: Address =
    aurora_engine_precompiles::make_address(0xe0f5206b, 0xbd039e7b0592d8918820024e2a7437b9);

impl TesterConstructor {
    pub fn load() -> Self {
        Self(solidity::ContractConstructor::compile_from_extended_json(
            "../etc/eth-contracts/artifacts/contracts/test/Tester.sol/Tester.json",
        ))
    }

    pub fn deploy(&self, nonce: u64, token: Address) -> LegacyEthTransaction {
        let data = self
            .0
            .abi
            .constructor()
            .unwrap()
            .encode_input(self.0.code.clone(), &[ethabi::Token::Address(token)])
            .unwrap();

        LegacyEthTransaction {
            nonce: nonce.into(),
            gas_price: Default::default(),
            gas: U256::from(DEPLOY_CONTRACT_GAS),
            to: None,
            value: Default::default(),
            data,
        }
    }
}

pub(crate) struct Tester {
    pub contract: solidity::DeployedContract,
}

impl From<TesterConstructor> for solidity::ContractConstructor {
    fn from(c: TesterConstructor) -> Self {
        c.0
    }
}

impl From<solidity::DeployedContract> for Tester {
    fn from(contract: solidity::DeployedContract) -> Self {
        Self { contract }
    }
}

impl Tester {
    fn call_function(
        &self,
        runner: &mut AuroraRunner,
        signer: &mut Signer,
        method: &str,
        value: Wei,
        params: &[ethabi::Token],
    ) -> Result<SubmitResult, Revert> {
        let data = self
            .contract
            .abi
            .function(method)
            .unwrap()
            .encode_input(params)
            .unwrap();

        let tx = LegacyEthTransaction {
            nonce: signer.use_nonce().into(),
            gas_price: Default::default(),
            gas: U256::from(DEPLOY_CONTRACT_GAS),
            to: Some(self.contract.address),
            value,
            data,
        };

        let result = runner.submit_transaction(&signer.secret_key, tx).unwrap();
        match result.status {
            aurora_engine::parameters::TransactionStatus::Succeed(_) => Ok(result),
            aurora_engine::parameters::TransactionStatus::Revert(bytes) => Err(Revert(bytes)),
            other => panic!("Unexpected status {:?}", other),
        }
    }

    pub fn hello_world(
        &self,
        runner: &mut AuroraRunner,
        signer: &mut Signer,
        name: String,
    ) -> String {
        let output_type = &[ethabi::ParamType::String];
        let result = self
            .call_function(
                runner,
                signer,
                "helloWorld",
                Wei::zero(),
                &[ethabi::Token::String(name)],
            )
            .unwrap();
        let output_bytes = test_utils::unwrap_success(result);
        let output = ethabi::decode(output_type, &output_bytes).unwrap();

        match &output[..] {
            [ethabi::Token::String(string)] => string.to_string(),
            _ => unreachable!(),
        }
    }

    pub fn withdraw(
        &self,
        runner: &mut AuroraRunner,
        signer: &mut Signer,
        flag: bool,
    ) -> Result<SubmitResult, Revert> {
        self.call_function(
            runner,
            signer,
            "withdraw",
            Wei::zero(),
            &[ethabi::Token::Bool(flag)],
        )
    }

    pub fn withdraw_and_fail(
        &self,
        runner: &mut AuroraRunner,
        signer: &mut Signer,
        flag: bool,
    ) -> Result<SubmitResult, Revert> {
        self.call_function(
            runner,
            signer,
            "withdrawAndFail",
            Wei::zero(),
            &[ethabi::Token::Bool(flag)],
        )
    }

    pub fn try_withdraw_and_avoid_fail(
        &self,
        runner: &mut AuroraRunner,
        signer: &mut Signer,
        flag: bool,
    ) -> Result<SubmitResult, Revert> {
        self.call_function(
            runner,
            signer,
            "tryWithdrawAndAvoidFail",
            Wei::zero(),
            &[ethabi::Token::Bool(flag)],
        )
    }

    pub fn try_withdraw_and_avoid_fail_and_succeed(
        &self,
        runner: &mut AuroraRunner,
        signer: &mut Signer,
        flag: bool,
    ) -> Result<SubmitResult, Revert> {
        self.call_function(
            runner,
            signer,
            "tryWithdrawAndAvoidFailAndSucceed",
            Wei::zero(),
            &[ethabi::Token::Bool(flag)],
        )
    }

    pub fn withdraw_eth(
        &self,
        runner: &mut AuroraRunner,
        signer: &mut Signer,
        is_to_near: bool,
        amount: Wei,
    ) -> Result<SubmitResult, Revert> {
        if is_to_near {
            self.call_function(
                runner,
                signer,
                "withdrawEthToNear",
                amount,
                &[ethabi::Token::Bytes(DEST_ACCOUNT.as_bytes().to_vec())],
            )
        } else {
            self.call_function(
                runner,
                signer,
                "withdrawEthToEthereum",
                amount,
                &[ethabi::Token::Address(DEST_ADDRESS)],
            )
        }
    }
}

#[derive(Debug)]
pub(crate) struct Revert(Vec<u8>);
