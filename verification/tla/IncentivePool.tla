---- MODULE IncentivePool ----
\* Value-conservation model of contracts/incentive-pool.
\* Tokens: poolBal is the pool's real token balance; deposited/paid mirror
\* TotalDeposited/TotalPaid. Bounded by MaxAmt and a small item/attester set.
EXTENDS Naturals, FiniteSets

CONSTANTS Items, Attesters, MaxAmt, MaxPerClaim, MaxPerAttester, MaxDeposit,
          BugDoublePay   \* TRUE injects: claim skips the already-claimed check

VARIABLES poolBal, deposited, paid, approved, claims, owner, amount, perAtt, paused

vars == <<poolBal, deposited, paid, approved, claims, owner, amount, perAtt, paused>>

Init == /\ poolBal = 0 /\ deposited = 0 /\ paid = 0
        /\ approved = {} /\ claims = [i \in Items |-> 0]
        /\ owner = [i \in Items |-> CHOOSE a \in Attesters : TRUE]
        /\ amount = [i \in Items |-> 0]
        /\ perAtt = [a \in Attesters |-> 0]
        /\ paused = FALSE

Fund(x) == /\ ~paused /\ x \in 1..MaxAmt /\ deposited + x <= MaxDeposit
           /\ poolBal' = poolBal + x /\ deposited' = deposited + x
           /\ UNCHANGED <<paid, approved, claims, owner, amount, perAtt, paused>>

Withdraw(x) == /\ ~paused /\ x \in 1..MaxAmt /\ x <= deposited - paid
               /\ poolBal' = poolBal - x /\ paid' = paid + x
               /\ UNCHANGED <<deposited, approved, claims, owner, amount, perAtt, paused>>

Approve(i, a, x) == /\ i \notin approved /\ x \in 1..MaxPerClaim
                    /\ approved' = approved \cup {i}
                    /\ owner' = [owner EXCEPT ![i] = a]
                    /\ amount' = [amount EXCEPT ![i] = x]
                    /\ UNCHANGED <<poolBal, deposited, paid, claims, perAtt, paused>>

Claim(i) == /\ ~paused /\ i \in approved
            /\ (BugDoublePay \/ claims[i] = 0)
            /\ perAtt[owner[i]] + amount[i] <= MaxPerAttester
            /\ paid + amount[i] <= deposited
            /\ poolBal' = poolBal - amount[i]
            /\ paid' = paid + amount[i]
            /\ claims' = [claims EXCEPT ![i] = @ + 1]
            /\ perAtt' = [perAtt EXCEPT ![owner[i]] = @ + amount[i]]
            /\ UNCHANGED <<deposited, approved, owner, amount, paused>>

SetPaused(b) == /\ paused' = b
                /\ UNCHANGED <<poolBal, deposited, paid, approved, claims, owner, amount, perAtt>>

Next == \/ \E x \in 1..MaxAmt : Fund(x) \/ Withdraw(x)
        \/ \E i \in Items, a \in Attesters, x \in 1..MaxPerClaim : Approve(i, a, x)
        \/ \E i \in Items : Claim(i)
        \/ \E b \in BOOLEAN : SetPaused(b)

Spec == Init /\ [][Next]_vars

\* Items paid out through `claim`.
Claimed == {i \in Items : claims[i] > 0}

RECURSIVE PaidOn(_)
PaidOn(S) == IF S = {} THEN 0
             ELSE LET i == CHOOSE j \in S : TRUE IN claims[i] * amount[i] + PaidOn(S \ {i})

\* Conservation: pool token balance equals deposits minus everything paid out.
Conservation == poolBal = deposited - paid /\ poolBal >= 0
\* Paid never exceeds deposited (no overdraw).
NoOverdraw == paid <= deposited
\* No double pay: every item is paid by `claim` at most once.
NoDoublePay == \A i \in Items : claims[i] <= 1
\* Claim payouts never exceed total paid (the rest is admin withdrawals).
PaidCoversClaims == PaidOn(Items) <= paid
\* Per-claim and per-attester caps.
Caps == /\ \A i \in approved : amount[i] <= MaxPerClaim
        /\ \A a \in Attesters : perAtt[a] <= MaxPerAttester
\* Claimed items are always approved.
ClaimedApproved == Claimed \subseteq approved
\* Per-attester total equals the sum of that attester's payouts.
AttesterAccounting == \A a \in Attesters :
    perAtt[a] = PaidOn({i \in Items : owner[i] = a})
====
