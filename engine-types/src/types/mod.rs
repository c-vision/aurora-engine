use crate::{str, vec, Address, String, Vec, U256};

pub mod address;
pub mod balance;
pub mod fee;
pub mod gas;
pub mod wei;

pub use balance::*;
pub use fee::*;
pub use gas::*;
pub use wei::*;

pub type RawAddress = [u8; 20];
pub type RawU256 = [u8; 32];
// Big-endian large integer type.
pub type RawH256 = [u8; 32]; // Unformatted binary data of fixed length.

pub type EthAddress = [u8; 20];
pub type StorageUsage = u64;

/// Selector to call mint function in ERC 20 contract
///
/// keccak("mint(address,uint256)".as_bytes())[..4];
#[allow(dead_code)]
pub const ERC20_MINT_SELECTOR: &[u8] = &[64, 193, 15, 25];

#[derive(Debug)]
pub enum AddressValidationError {
    FailedDecodeHex,
    IncorrectLength,
}

impl AsRef<[u8]> for AddressValidationError {
    fn as_ref(&self) -> &[u8] {
        match self {
            Self::FailedDecodeHex => b"FAILED_DECODE_ETH_ADDRESS",
            Self::IncorrectLength => b"ETH_WRONG_ADDRESS_LENGTH",
        }
    }
}

/// Validate Ethereum address from string and return Result data EthAddress or Error data
pub fn validate_eth_address(address: String) -> Result<EthAddress, AddressValidationError> {
    let data = hex::decode(address).map_err(|_| AddressValidationError::FailedDecodeHex)?;
    if data.len() != 20 {
        return Err(AddressValidationError::IncorrectLength);
    }
    let mut result = [0u8; 20];
    result.copy_from_slice(&data);
    Ok(result)
}

pub const STORAGE_PRICE_PER_BYTE: u128 = 10_000_000_000_000_000_000;
// 1e19yN, 0.00001N
pub const ERR_FAILED_PARSE: &str = "ERR_FAILED_PARSE";
pub const ERR_INVALID_ETH_ADDRESS: &str = "ERR_INVALID_ETH_ADDRESS";

/// Internal args format for meta call.
#[derive(Debug)]
pub struct InternalMetaCallArgs {
    pub sender: Address,
    pub nonce: U256,
    pub fee_amount: Wei,
    pub fee_address: Address,
    pub contract_address: Address,
    pub value: Wei,
    pub input: Vec<u8>,
}

pub struct StorageBalanceBounds {
    pub min: Balance,
    pub max: Option<Balance>,
}

/// promise results structure
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromiseResult {
    NotReady,
    Successful(Vec<u8>),
    Failed,
}

/// ft_resolve_transfer result of eth-connector
pub struct FtResolveTransferResult {
    pub amount: Balance,
    pub refund_amount: Balance,
}

/// Internal errors to propagate up and format in the single place.
pub enum ErrorKind {
    ArgumentParseError,
    InvalidMetaTransactionMethodName,
    InvalidMetaTransactionFunctionArg,
    InvalidEcRecoverSignature,
}

const HEX_ALPHABET: &[u8; 16] = b"0123456789abcdef";

#[allow(dead_code)]
pub fn bytes_to_hex(v: &[u8]) -> String {
    let mut result = String::new();
    for x in v {
        result.push(HEX_ALPHABET[(x / 16) as usize] as char);
        result.push(HEX_ALPHABET[(x % 16) as usize] as char);
    }
    result
}

#[derive(Default)]
pub struct Stack<T> {
    stack: Vec<T>,
    boundaries: Vec<usize>,
}

impl<T> Stack<T> {
    pub fn new() -> Self {
        Self {
            stack: Vec::new(),
            boundaries: vec![0],
        }
    }

    pub fn enter(&mut self) {
        self.boundaries.push(self.stack.len());
    }

    pub fn commit(&mut self) {
        self.boundaries.pop().unwrap();
    }

    pub fn discard(&mut self) {
        let boundary = self.boundaries.pop().unwrap();
        self.stack.truncate(boundary);
    }

    pub fn push(&mut self, value: T) {
        self.stack.push(value);
    }

    pub fn into_vec(self) -> Vec<T> {
        self.stack
    }
}

pub fn str_from_slice(inp: &[u8]) -> &str {
    str::from_utf8(inp).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hex() {
        assert_eq!(
            bytes_to_hex(&[0u8, 1u8, 255u8, 16u8]),
            "0001ff10".to_string()
        );
    }

    /// Build view of the stack. Intervals between None values are scopes.
    fn view_stack(stack: &Stack<i32>) -> Vec<Option<i32>> {
        let mut res = vec![];
        let mut pnt = 0;

        for &pos in stack.boundaries.iter() {
            while pnt < pos {
                res.push(Some(stack.stack[pnt]));
                pnt += 1;
            }
            res.push(None);
        }

        while pnt < stack.stack.len() {
            res.push(Some(stack.stack[pnt]));
            pnt += 1;
        }

        res
    }

    fn check_stack(stack: &Stack<i32>, expected: Vec<Option<i32>>) {
        if let Some(&last) = stack.boundaries.last() {
            assert!(last <= stack.stack.len());
        }
        assert_eq!(view_stack(stack), expected);
    }

    #[test]
    fn test_stack() {
        let mut stack = Stack::new(); // [ $ ]
        check_stack(&stack, vec![None]);

        stack.push(1); // [ $, 1]
        check_stack(&stack, vec![None, Some(1)]);
        stack.push(2); // [ $, 1, 2 ]
        check_stack(&stack, vec![None, Some(1), Some(2)]);
        stack.enter(); // [$, 1, 2, $]
        check_stack(&stack, vec![None, Some(1), Some(2), None]);
        stack.push(3); // [$, 1, 2, $, 3]
        check_stack(&stack, vec![None, Some(1), Some(2), None, Some(3)]);
        stack.discard(); // [$, 1, 2]
        check_stack(&stack, vec![None, Some(1), Some(2)]);
        stack.enter();
        check_stack(&stack, vec![None, Some(1), Some(2), None]);
        stack.push(4); // [$, 1, 2, $, 4]
        check_stack(&stack, vec![None, Some(1), Some(2), None, Some(4)]);
        stack.enter(); // [$, 1, 2, $, 4, $]
        check_stack(&stack, vec![None, Some(1), Some(2), None, Some(4), None]);
        stack.push(5); // [$, 1, 2, $, 4, $, 5]
        check_stack(
            &stack,
            vec![None, Some(1), Some(2), None, Some(4), None, Some(5)],
        );
        stack.commit(); // [$, 1, 2, $, 4, 5]
        check_stack(&stack, vec![None, Some(1), Some(2), None, Some(4), Some(5)]);
        stack.discard(); // [$, 1, 2]
        check_stack(&stack, vec![None, Some(1), Some(2)]);
        stack.push(6); // [$, 1, 2, 6]
        check_stack(&stack, vec![None, Some(1), Some(2), Some(6)]);
        stack.enter(); // [$, 1, 2, 6, $]
        check_stack(&stack, vec![None, Some(1), Some(2), Some(6), None]);
        stack.enter(); // [$, 1, 2, 6, $, $]
        check_stack(&stack, vec![None, Some(1), Some(2), Some(6), None, None]);
        stack.enter(); // [$, 1, 2, 6, $, $, $]
        check_stack(
            &stack,
            vec![None, Some(1), Some(2), Some(6), None, None, None],
        );
        stack.commit(); // [$, 1, 2, 6, $, $]
        check_stack(&stack, vec![None, Some(1), Some(2), Some(6), None, None]);
        stack.discard(); // [$, 1, 2, 6, $]
        check_stack(&stack, vec![None, Some(1), Some(2), Some(6), None]);
        stack.push(7); // [$, 1, 2, 6, $, 7]

        assert_eq!(stack.into_vec(), vec![1, 2, 6, 7]);
    }
}
