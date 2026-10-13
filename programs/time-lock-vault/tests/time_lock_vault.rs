use {
    litesvm::{types::TransactionResult, LiteSVM},
    solana_address::Address,
    solana_clock::Clock,
    solana_instruction::{AccountMeta, Instruction},
    solana_instruction_error::InstructionError,
    solana_keypair::Keypair,
    solana_signer::Signer,
    solana_system_interface::instruction as system_instruction,
    solana_transaction::Transaction,
    solana_transaction_error::TransactionError,
};

const TOKEN_PROGRAM: Address = Address::from_str_const("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA");
const SYSTEM_PROGRAM: Address = Address::from_str_const("11111111111111111111111111111111");

const ERR_ZERO_AMOUNT: u32 = 0;
const ERR_UNLOCK_NOT_IN_FUTURE: u32 = 1;
const ERR_STILL_LOCKED: u32 = 2;
const ERR_NOT_LOCK_OWNER: u32 = 3;
const ERR_INVALID_ACCOUNT: u32 = 4;

const START: i64 = 1_000;
const UNLOCK_AT: i64 = 5_000;
const MINTED: u64 = 1_000;

struct Env {
    svm: LiteSVM,
    program_id: Address,
    payer: Keypair,
    owner: Keypair,
    mint: Address,
    owner_token: Address,
}

impl Env {
    fn new() -> Self {
        let mut svm = LiteSVM::new();
        let program_id = Address::new_unique();
        svm.add_program_from_file(
            program_id,
            concat!(env!("CARGO_MANIFEST_DIR"), "/target/deploy/time_lock_vault.so"),
        )
        .expect("build the program first with `cargo build-sbf`");

        let payer = Keypair::new();
        let owner = Keypair::new();
        svm.airdrop(&payer.pubkey(), 10_000_000_000).unwrap();
        svm.airdrop(&owner.pubkey(), 10_000_000_000).unwrap();

        let mut env = Self {
            svm,
            program_id,
            payer,
            owner,
            mint: Address::default(),
            owner_token: Address::default(),
        };
        env.set_time(START);
        env.mint = env.create_mint(6);
        let owner_pk = env.owner.pubkey();
        env.owner_token = env.create_token_account(&env.mint.clone(), &owner_pk);
        env.mint_to(&env.mint.clone(), &env.owner_token.clone(), MINTED);
        env
    }

    fn set_time(&mut self, unix_timestamp: i64) {
        let mut clock = self.svm.get_sysvar::<Clock>();
        clock.unix_timestamp = unix_timestamp;
        self.svm.set_sysvar(&clock);
    }

    fn send(&mut self, ixs: &[Instruction], signers: &[&Keypair]) -> TransactionResult {
        self.svm.expire_blockhash();
        let tx = Transaction::new_signed_with_payer(
            ixs,
            Some(&signers[0].pubkey()),
            signers,
            self.svm.latest_blockhash(),
        );
        self.svm.send_transaction(tx)
    }

    fn send_setup(&mut self, ixs: &[Instruction], extra: &[&Keypair]) {
        let payer = self.payer.insecure_clone();
        let mut signers = vec![&payer];
        signers.extend_from_slice(extra);
        self.send(ixs, &signers).expect("setup transaction failed");
    }

    fn create_mint(&mut self, decimals: u8) -> Address {
        let mint = Keypair::new();
        let rent = self.svm.minimum_balance_for_rent_exemption(82);
        let create =
            system_instruction::create_account(&self.payer.pubkey(), &mint.pubkey(), rent, 82, &TOKEN_PROGRAM);
        let mut data = vec![20u8, decimals];
        data.extend_from_slice(self.payer.pubkey().as_ref());
        data.push(0);
        let init = Instruction {
            program_id: TOKEN_PROGRAM,
            accounts: vec![AccountMeta::new(mint.pubkey(), false)],
            data,
        };
        self.send_setup(&[create, init], &[&mint]);
        mint.pubkey()
    }

    fn create_token_account(&mut self, mint: &Address, owner: &Address) -> Address {
        let account = Keypair::new();
        let rent = self.svm.minimum_balance_for_rent_exemption(165);
        let create = system_instruction::create_account(
            &self.payer.pubkey(),
            &account.pubkey(),
            rent,
            165,
            &TOKEN_PROGRAM,
        );
        let mut data = vec![18u8];
        data.extend_from_slice(owner.as_ref());
        let init = Instruction {
            program_id: TOKEN_PROGRAM,
            accounts: vec![
                AccountMeta::new(account.pubkey(), false),
                AccountMeta::new_readonly(*mint, false),
            ],
            data,
        };
        self.send_setup(&[create, init], &[&account]);
        account.pubkey()
    }

    fn mint_to(&mut self, mint: &Address, dest: &Address, amount: u64) {
        let mut data = vec![7u8];
        data.extend_from_slice(&amount.to_le_bytes());
        let ix = Instruction {
            program_id: TOKEN_PROGRAM,
            accounts: vec![
                AccountMeta::new(*mint, false),
                AccountMeta::new(*dest, false),
                AccountMeta::new_readonly(self.payer.pubkey(), true),
            ],
            data,
        };
        self.send_setup(&[ix], &[]);
    }

    fn lock_pda(&self, owner: &Address) -> Address {
        Address::find_program_address(&[b"lock", owner.as_ref(), self.mint.as_ref()], &self.program_id).0
    }

    fn vault_pda(&self, owner: &Address) -> Address {
        let lock = self.lock_pda(owner);
        Address::find_program_address(&[b"vault", lock.as_ref()], &self.program_id).0
    }

    fn lock_ix(&self, owner: &Address, owner_token: &Address, amount: u64, unlock_at: i64) -> Instruction {
        let mut data = vec![0u8];
        data.extend_from_slice(&amount.to_le_bytes());
        data.extend_from_slice(&unlock_at.to_le_bytes());
        Instruction {
            program_id: self.program_id,
            accounts: vec![
                AccountMeta::new(*owner, true),
                AccountMeta::new(*owner_token, false),
                AccountMeta::new_readonly(self.mint, false),
                AccountMeta::new(self.lock_pda(owner), false),
                AccountMeta::new(self.vault_pda(owner), false),
                AccountMeta::new_readonly(SYSTEM_PROGRAM, false),
                AccountMeta::new_readonly(TOKEN_PROGRAM, false),
            ],
            data,
        }
    }

    /// `lock_owner` selects whose lock PDAs are used; `signer` is who signs and receives the rent.
    fn withdraw_ix(
        &self,
        lock_owner: &Address,
        signer: &Address,
        destination: &Address,
    ) -> Instruction {
        Instruction {
            program_id: self.program_id,
            accounts: vec![
                AccountMeta::new(*signer, true),
                AccountMeta::new(*destination, false),
                AccountMeta::new_readonly(self.mint, false),
                AccountMeta::new(self.lock_pda(lock_owner), false),
                AccountMeta::new(self.vault_pda(lock_owner), false),
                AccountMeta::new_readonly(TOKEN_PROGRAM, false),
            ],
            data: vec![1u8],
        }
    }

    fn lock(&mut self, amount: u64, unlock_at: i64) -> TransactionResult {
        let owner = self.owner.insecure_clone();
        let ix = self.lock_ix(&owner.pubkey(), &self.owner_token.clone(), amount, unlock_at);
        self.send(&[ix], &[&owner])
    }

    fn withdraw(&mut self) -> TransactionResult {
        let owner = self.owner.insecure_clone();
        let ix = self.withdraw_ix(&owner.pubkey(), &owner.pubkey(), &self.owner_token.clone());
        self.send(&[ix], &[&owner])
    }

    fn token_balance(&self, account: &Address) -> u64 {
        let data = self.svm.get_account(account).expect("token account missing").data;
        u64::from_le_bytes(data[64..72].try_into().unwrap())
    }

    fn account_exists(&self, address: &Address) -> bool {
        self.svm
            .get_account(address)
            .map(|a| a.lamports > 0)
            .unwrap_or(false)
    }
}

fn assert_custom_error(result: TransactionResult, code: u32) {
    match result {
        Err(failed) => assert_eq!(
            failed.err,
            TransactionError::InstructionError(0, InstructionError::Custom(code)),
            "logs: {:?}",
            failed.meta.logs
        ),
        Ok(_) => panic!("expected custom error {code}, but the transaction succeeded"),
    }
}

fn assert_instruction_error(result: TransactionResult, expected: InstructionError) {
    match result {
        Err(failed) => assert_eq!(
            failed.err,
            TransactionError::InstructionError(0, expected),
            "logs: {:?}",
            failed.meta.logs
        ),
        Ok(_) => panic!("expected {expected:?}, but the transaction succeeded"),
    }
}

#[test]
fn lock_then_withdraw_after_unlock_time_works() {
    let mut env = Env::new();
    let owner = env.owner.pubkey();
    let (lock, vault) = (env.lock_pda(&owner), env.vault_pda(&owner));

    env.lock(400, UNLOCK_AT).expect("lock failed");
    assert_eq!(env.token_balance(&env.owner_token), MINTED - 400);
    assert_eq!(env.token_balance(&vault), 400);
    assert!(env.account_exists(&lock));

    let owner_lamports_before = env.svm.get_balance(&owner).unwrap();
    env.set_time(UNLOCK_AT);
    env.withdraw().expect("withdraw failed");

    assert_eq!(env.token_balance(&env.owner_token), MINTED);
    assert!(!env.account_exists(&vault));
    assert!(!env.account_exists(&lock));
    assert!(
        env.svm.get_balance(&owner).unwrap() > owner_lamports_before,
        "rent of both accounts should be returned to the owner"
    );
}

#[test]
fn withdraw_before_unlock_time_fails() {
    let mut env = Env::new();
    let vault = env.vault_pda(&env.owner.pubkey());
    env.lock(400, UNLOCK_AT).expect("lock failed");

    env.set_time(UNLOCK_AT - 1);
    assert_custom_error(env.withdraw(), ERR_STILL_LOCKED);
    assert_eq!(env.token_balance(&vault), 400);
    assert_eq!(env.token_balance(&env.owner_token), MINTED - 400);

    env.set_time(UNLOCK_AT);
    env.withdraw().expect("withdraw at unlock time failed");
    assert_eq!(env.token_balance(&env.owner_token), MINTED);
}

#[test]
fn stranger_cannot_withdraw() {
    let mut env = Env::new();
    let owner = env.owner.pubkey();
    let vault = env.vault_pda(&owner);
    env.lock(400, UNLOCK_AT).expect("lock failed");
    env.set_time(UNLOCK_AT + 1);

    let stranger = Keypair::new();
    env.svm.airdrop(&stranger.pubkey(), 1_000_000_000).unwrap();
    let mint = env.mint;
    let stranger_token = env.create_token_account(&mint, &stranger.pubkey());

    // Stranger signs as themselves and points at the owner's lock.
    let ix = env.withdraw_ix(&owner, &stranger.pubkey(), &stranger_token);
    assert_custom_error(env.send(&[ix], &[&stranger]), ERR_NOT_LOCK_OWNER);

    // Stranger names the real owner but cannot sign for them.
    let ix = env.withdraw_ix(&owner, &stranger.pubkey(), &stranger_token);
    let mut ix_unsigned_owner = ix.clone();
    ix_unsigned_owner.accounts[0] = AccountMeta::new(owner, false);
    assert!(env.send(&[ix_unsigned_owner], &[&stranger]).is_err());

    // Even with the owner's signature, funds can only go to the owner's own token account.
    let owner_kp = env.owner.insecure_clone();
    let ix = env.withdraw_ix(&owner, &owner, &stranger_token);
    assert_custom_error(env.send(&[ix], &[&owner_kp]), ERR_INVALID_ACCOUNT);

    // Stranger using their own (non-existent) lock fails too.
    let ix = env.withdraw_ix(&stranger.pubkey(), &stranger.pubkey(), &stranger_token);
    assert!(env.send(&[ix], &[&stranger]).is_err());

    assert_eq!(env.token_balance(&vault), 400);
    assert_eq!(env.token_balance(&stranger_token), 0);
}

#[test]
fn locking_zero_fails() {
    let mut env = Env::new();
    assert_custom_error(env.lock(0, UNLOCK_AT), ERR_ZERO_AMOUNT);
    assert!(!env.account_exists(&env.lock_pda(&env.owner.pubkey())));
    assert_eq!(env.token_balance(&env.owner_token), MINTED);
}

#[test]
fn unlock_time_must_be_in_the_future() {
    let mut env = Env::new();
    assert_custom_error(env.lock(100, START), ERR_UNLOCK_NOT_IN_FUTURE);
    assert_custom_error(env.lock(100, START - 1), ERR_UNLOCK_NOT_IN_FUTURE);
}

#[test]
fn cannot_lock_twice_for_same_mint() {
    let mut env = Env::new();
    env.lock(100, UNLOCK_AT).expect("first lock failed");
    assert!(env.lock(100, UNLOCK_AT + 1).is_err());
    let vault = env.vault_pda(&env.owner.pubkey());
    assert_eq!(env.token_balance(&vault), 100);
}

#[test]
fn cannot_lock_more_than_balance() {
    let mut env = Env::new();
    assert!(env.lock(MINTED + 1, UNLOCK_AT).is_err());
    assert!(!env.account_exists(&env.lock_pda(&env.owner.pubkey())));
}

#[test]
fn lock_requires_owner_signature() {
    let mut env = Env::new();
    let owner = env.owner.pubkey();
    let payer = env.payer.insecure_clone();
    let mut ix = env.lock_ix(&owner, &env.owner_token.clone(), 100, UNLOCK_AT);
    ix.accounts[0] = AccountMeta::new(owner, false);
    assert_instruction_error(
        env.send(&[ix], &[&payer]),
        InstructionError::MissingRequiredSignature,
    );
}

#[test]
fn withdraw_sweeps_tokens_donated_to_the_vault() {
    let mut env = Env::new();
    let owner = env.owner.pubkey();
    let vault = env.vault_pda(&owner);
    env.lock(400, UNLOCK_AT).expect("lock failed");

    let (mint, owner_token) = (env.mint, env.owner_token);
    env.mint_to(&mint, &vault, 50);
    env.set_time(UNLOCK_AT);
    env.withdraw().expect("withdraw failed");
    assert_eq!(env.token_balance(&owner_token), MINTED + 50);
}

#[test]
fn can_lock_again_after_withdrawing() {
    let mut env = Env::new();
    env.lock(400, UNLOCK_AT).expect("lock failed");
    env.set_time(UNLOCK_AT);
    env.withdraw().expect("withdraw failed");

    env.lock(MINTED, UNLOCK_AT + 100).expect("second lock failed");
    assert_eq!(env.token_balance(&env.owner_token), 0);
    env.set_time(UNLOCK_AT + 100);
    env.withdraw().expect("second withdraw failed");
    assert_eq!(env.token_balance(&env.owner_token), MINTED);
}
