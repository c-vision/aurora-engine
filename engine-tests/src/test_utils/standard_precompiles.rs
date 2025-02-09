use crate::prelude::{transaction::LegacyEthTransaction, U256};
use crate::test_utils::solidity;
use std::path::{Path, PathBuf};

pub(crate) struct PrecompilesConstructor(pub solidity::ContractConstructor);

pub(crate) struct PrecompilesContract(pub solidity::DeployedContract);

impl From<PrecompilesConstructor> for solidity::ContractConstructor {
    fn from(c: PrecompilesConstructor) -> Self {
        c.0
    }
}

impl PrecompilesConstructor {
    pub fn load() -> Self {
        Self(solidity::ContractConstructor::compile_from_source(
            Self::sources_root(),
            Self::solidity_artifacts_path(),
            "StandardPrecompiles.sol",
            "StandardPrecompiles",
        ))
    }

    pub fn deploy(&self, nonce: U256) -> LegacyEthTransaction {
        self.0.deploy_without_args(nonce)
    }

    fn solidity_artifacts_path() -> PathBuf {
        Path::new("target").join("solidity_build")
    }

    fn sources_root() -> PathBuf {
        Path::new("src").join("benches").join("res")
    }
}

impl PrecompilesContract {
    pub fn call_method(&self, method_name: &str, nonce: U256) -> LegacyEthTransaction {
        self.0.call_method_without_args(method_name, nonce)
    }

    pub fn all_method_names() -> &'static [&'static str] {
        &[
            "test_ecrecover",
            "test_sha256",
            "test_ripemd160",
            "test_identity",
            "test_modexp",
            "test_ecadd",
            "test_ecmul",
            // TODO(#46): ecpair uses up all the gas (by itself) for some reason, need to look into this.
            // "test_ecpair",
            "test_blake2f",
            "test_all",
        ]
    }
}
