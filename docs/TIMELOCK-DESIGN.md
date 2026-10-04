# Timelock Controller Contract Design

**Issue:** #369 — Build a generic timelock controller contract for delayed, cancellable admin operations

## Overview

Every admin action in the registries takes effect immediately: removing attesters, transferring admin, pausing/unpausing, changing configuration. If an admin key or quorum is compromised, the attacker's changes land in the same ledger. The community, CHW programme leads, and monitoring systems have zero opportunity to detect or react.

A timelock contract is the standard defense: it intercepts admin operations, delays their execution by a configurable period, and provides a cancellation window so authorized guardians can prevent malicious changes.

## Architecture

The timelock acts as a "gatekeeper" that becomes the admin of the registries. When a change is needed, the multisig (or another authorized proposer) schedules it through the timelock, and it executes after a delay.

### Core Concepts

**Proposer Role:** Submits operations to be scheduled. Typically the multisig.

**Canceller Role:** Can cancel a scheduled operation before it executes. May be a separate entity (e.g., a guardian, audit committee, or broader multisig).

**Operation ID:** SHA-256(target_contract || function_name || args || salt). Deterministic; same operation with same inputs has the same ID.

**Delay Period:** Configurable per operation type. Examples:
- `pause`: 0 ledgers (emergency, needs immediate effect)
- `add_attester`: 604,800 ledgers (~7 days)
- `transfer_admin`: 2,419,200 ledgers (~28 days)
- `upgrade`: 2,419,200 ledgers (~28 days)

### Contract Interface

```rust
#[contracttype]
pub struct ScheduledOperation {
    pub op_id: BytesN<32>,
    pub target: Address,
    pub function: Symbol,
    pub args: Vec<Val>,
    pub delay: u32,
    pub scheduled_at: u64,
    pub eta: u64,  // scheduled_at + delay (in ledger seconds)
}

pub fn schedule(
    env: Env,
    target: Address,
    function: Symbol,
    args: Vec<Val>,
    delay: u32,
) -> Result<BytesN<32>, Error> {
    // Verify proposer auth
    let proposer = Self::proposer(&env)?;
    proposer.require_auth();

    // Validate delay >= min_delay for this function
    let min_delay = Self::min_delay_for_function(&function);
    if delay < min_delay {
        return Err(Error::DelayTooShort);
    }

    // Compute operation ID
    let salt = env.ledger().sequence();
    let op_id = compute_op_id(&target, &function, &args, salt);

    // Check operation not already scheduled
    if env.storage().instance().has(&DataKey::ScheduledOp(op_id)) {
        return Err(Error::OperationAlreadyScheduled);
    }

    // Store scheduled operation
    let now = env.ledger().timestamp();
    let eta = now + (delay as u64 * 5);  // ~5 seconds per ledger
    let op = ScheduledOperation {
        op_id,
        target,
        function,
        args,
        delay,
        scheduled_at: now,
        eta,
    };
    env.storage().instance().set(&DataKey::ScheduledOp(op_id), &op);

    env.events().publish(
        (Symbol::new(&env, "OperationScheduled"),),
        (op_id, target, function, eta),
    );

    Ok(op_id)
}

pub fn execute(env: Env, op_id: BytesN<32>) -> Result<(), Error> {
    // Any address can call execute; timing is the only guard
    let op: ScheduledOperation = env.storage().instance()
        .get(&DataKey::ScheduledOp(op_id))
        .ok_or(Error::OperationNotFound)?;

    // Check delay has passed
    let now = env.ledger().timestamp();
    if now < op.eta {
        return Err(Error::DelayNotMet);
    }

    // Check not already executed
    if env.storage().instance().has(&DataKey::ExecutedOp(op_id)) {
        return Err(Error::OperationAlreadyExecuted);
    }

    // Invoke target contract with timelock as the authorizing signer
    // (This is the key: the registries think the timelock is calling them)
    env.invoke_contract::<()>(
        &op.target,
        &op.function,
        op.args,
    );

    // Mark as executed
    env.storage().instance().set(&DataKey::ExecutedOp(op_id), &true);
    env.storage().instance().remove(&DataKey::ScheduledOp(op_id));

    env.events().publish(
        (Symbol::new(&env, "OperationExecuted"),),
        (op_id, op.target, op.function),
    );

    Ok(())
}

pub fn cancel(env: Env, op_id: BytesN<32>) -> Result<(), Error> {
    // Verify canceller auth
    let canceller = Self::canceller(&env)?;
    canceller.require_auth();

    let op: ScheduledOperation = env.storage().instance()
        .get(&DataKey::ScheduledOp(op_id))
        .ok_or(Error::OperationNotFound)?;

    // Check not already executed
    if env.storage().instance().has(&DataKey::ExecutedOp(op_id)) {
        return Err(Error::OperationAlreadyExecuted);
    }

    env.storage().instance().remove(&DataKey::ScheduledOp(op_id));

    env.events().publish(
        (Symbol::new(&env, "OperationCancelled"),),
        (op_id, op.target, op.function),
    );

    Ok(())
}

pub fn get_scheduled(env: Env, op_id: BytesN<32>) -> Option<ScheduledOperation> {
    env.storage().instance().get(&DataKey::ScheduledOp(op_id))
}
```

### Per-Function Delay Overrides

Each function registered with the timelock has a minimum delay:

```rust
fn min_delay_for_function(function: &Symbol) -> u32 {
    match function.to_string().as_str() {
        "pause" => 0,              // Emergency: no delay
        "unpause" => 0,            // Emergency: no delay
        "add_attester" => 604_800, // ~7 days
        "remove_attester" => 1_209_600, // ~14 days
        "transfer_admin" => 2_419_200,  // ~28 days
        "upgrade" => 2_419_200,    // ~28 days
        _ => 1_209_600,            // Default: ~14 days
    }
}
```

Admin can override per-function delays (with delay):

```rust
pub fn set_function_delay(env: Env, function: Symbol, delay: u32) -> Result<(), Error> {
    Self::admin(&env)?.require_auth();
    env.storage().instance().set(&DataKey::FunctionDelay(function), &delay);
    Ok(())
}
```

### Integration with Registries

The registries' admin is set to the timelock contract address. When an operation needs to happen:

1. **Before:** Admin is a multisig; changes execute immediately on multisig signature
2. **After:** Admin is the timelock; changes go through schedule → wait → execute

Example call from the multisig:
```rust
// Step 1: Schedule through timelock
let op_id = timelock.schedule(
    attester_registry,
    Symbol::new(&env, "add_attester"),
    vec![attester_address],
    604_800,  // ~7 days
)?;

// Step 2: Wait ~7 days
// (monitoring systems, CHWs, and guardians watch the scheduled operation)

// Step 3: Execute (can be called by anyone after delay)
timelock.execute(op_id)?;  // Adds attester to registry
```

### Events

- **OperationScheduled:** `(op_id, target, function, eta)`
- **OperationExecuted:** `(op_id, target, function)`
- **OperationCancelled:** `(op_id, target, function)`

Off-chain monitoring can subscribe to these events to alert stakeholders of pending changes.

## Security Considerations

### Preventing Re-Entrancy
- Once executed, `op_id` is marked in `ExecutedOp` set; can't be executed twice
- New operations always get a fresh salt (ledger sequence), preventing ID collisions

### Preventing Bypass
- Timelock must be the sole admin of registries
- If the timelock is compromised, all admin functions are compromised
- This is acceptable: the timelock is narrow and auditable

### Cancellation Race Condition
- Between the moment delay expires and execution is called, the canceller could cancel
- This is intentional: it's the "escape hatch" for catching malicious operations
- Off-chain coordination (monitoring → alert → canceller acts) must be fast

## Operational Workflow

### Setup
1. Deploy timelock contract
2. Set timelock address as admin of `attester-registry` and `attestation-registry`
3. Configure proposer (multisig) and canceller (guardian) addresses
4. Register public function names and their minimum delays

### Day-to-Day
1. Multisig proposes an operation (e.g., add attester) via `schedule()`
2. Monitoring systems and guardians see the event and verify the operation is benign
3. After delay, proposer or anyone calls `execute()`
4. If guardians detect a malicious operation, canceller calls `cancel()` before ETA

### Emergency (Pause Needed)
- Pause has 0-delay; can be executed immediately after scheduling
- Allows legitimate emergencies (e.g., fraudulent attestations detected) to be stopped quickly
- Paired with communication to stakeholders

## Implementation Phases

### Phase 1: Core Timelock
- Implement `schedule`, `execute`, `cancel`, `get_scheduled`
- Fixed per-function delays
- No admin override capability

### Phase 2: Operational Integration
- Deploy timelock contract
- Set as admin of both registries
- Coordinate with monitoring systems to watch events
- Test with add_attester, remove_attester operations

### Phase 3: Dynamic Delays
- Implement `set_function_delay()` with admin override
- Per-contract delay registration
- Document delay policy in runbook

### Phase 4: Analytics
- Dashboard showing scheduled operations, time to execution, historical cancellations
- Helps community assess operational health and trust in the system

## References

- Issue #369: Timelock controller for delayed admin operations
- Standard pattern: OpenZeppelin TimelockController: https://docs.openzeppelin.com/contracts/latest/api/governance#TimelockController
- Soroban invoke_contract: https://stellar.org/docs/build/smart-contracts/interacting-with-other-contracts
