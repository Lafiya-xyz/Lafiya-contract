# CHW trustline onboarding (incentive-pool payouts)

`incentive-pool::claim` pays the attester in the pool token (USDC, a Stellar
Asset Contract). A recipient account without a trustline, or whose trustline
the issuer has not authorized, cannot receive it. Since #509 such a claim
returns `Error::RecipientCannotReceive` (code `16`), writes no state, and the
approval stays claimable.

## Onboarding step

1. When a CHW is added to `attester-registry`, their wallet must submit a
   `ChangeTrust` operation for the USDC asset (classic Stellar accounts), or
   be a contract address (which needs no trustline).
2. If the issuer uses `AUTH_REQUIRED`, request authorization from the issuer.
3. Verify with the pool's preflight view before the first claim:

   ```sh
   stellar contract invoke --id <POOL_ID> -- can_receive --address <CHW_ADDRESS>
   ```

   `true` means the claim transfer will succeed.

## Recovering a failed claim

On `RecipientCannotReceive`, complete steps 1-3 above and retry `claim` with
the same work-item id; nothing needs to be re-approved.
