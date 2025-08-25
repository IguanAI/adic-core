# ADIC Phase‑0 Testnet — Production‑Ready Design v0.3 (Vue; Trust Rationale + Calibration Checkpoint)

**Status:** Draft v0.3 (final polish after core‑dev review)
**Scope:** Production‑like Phase‑0 design with clarified MRW trust rationale and a formal parameter‑calibration checkpoint; **Vue 3 + Vite** explorer
**Owner:** ADIC Core (Protocol · Node · Ops · Explorer)
**Languages & Stacks:** Rust (node/services), **TypeScript + Vue 3 + Vite** (Explorer), Python (sims/oracles)
**External math lib:** libadic (C++/Py) for sims/oracles; node uses a lean Rust p‑adic surface (vp, vp\_diff, ball\_id)

---

## 0) Executive summary

Phase‑0 delivers a public, permissionless ADIC DAG testnet with **Admissibility (C1–C3)**, **MRW parent selection** (with proximity, trust, and conflict penalty), and **F1 (k‑core) finality** as the default gate. We ship a **single node binary**, a **Vue‑based explorer**, and a **simulation harness**. Emphasis: correctness, observability, determinism, and production‑grade ops. Optional gates **F2 (PH)** and **SSF** remain feature‑flagged **off** by default.

**Default parameters (tunable):**
`p=3, d=3, ρ=(2,2,1), q=3, k=20, D*=12, Δ=5, deposit D=0.1, r_min=1.0, R_sum_min=4.0, λ=1.0, β=0.5, μ=1.0`
(Phase‑0 defaults; calibrated in sims and confirmed during the T‑3/T‑4 **Parameter Calibration Review**.)

---

## 1) Goals & non‑goals

### 1.1 Goals

* **Protocol correctness (Phase‑0):** `d+1` parents; C1–C3 via p‑adic balls and static rep floors; MRW with proximity/trust/penalty; F1 finality; anti‑spam deposits.
* **Production‑like ops:** packaged binary, Prometheus/Grafana, structured logs, recovery/snapshots, signed releases.
* **Observability:** MRW traces, axis diversity, k‑core growth, conflict energy, penalty usage, C3 floors pass/fail.
* **Determinism:** frozen `/specs/`, reproducible snapshots, CI property + fuzz + integration tests.

### 1.2 Non‑goals (deferred)

* **F2 Persistent Homology Finality:** The F2 finality gate, based on topological data analysis, is feature-flagged off by default in Phase-0. The focus is on the simpler and more deterministic F1 (k-core) gate.
* **Axis-aware Vector Clocks:** The whitepaper mentions vector clocks per axis for a fine-grained acyclicity check. The current implementation uses a simpler DAG ancestor check. The more complex clock mechanism is deferred.
* **Dynamic Reputation Learning:** The reputation score (`ADIC-Rep`) is updated based on finality events, but the initial implementation does not include more advanced dynamic learning or decay mechanisms based on network-wide activity.
* **Token Economics:** The full utility and governance model of the ADIC token is out of scope for the Phase-0 node implementation, which only concerns itself with the anti-spam deposits.
* **Smart-contract Layer:** No smart-contract or general-purpose computation layer is included in this phase.

---

## 2) Architecture overview

### 2.1 Components

* **`adic-node`** (Rust): consensus, storage, networking, APIs.
* **`adic-explorer-api`** (Rust or TS): read‑only REST/GraphQL over node/read‑replicas.
* **`ui-explorer`** (**Vue 3 + Vite**): Live DAG, MRW traces, finality monitor, conflict energy, params panel.
* **`adic-sim`** (Python): Monte Carlo workloads; libadic oracles for cross‑checks.

### 2.2 Boundaries & streams

* Node exposes **gRPC** (internal) + **HTTP/JSON** and **SSE** (public). Explorer consumes read APIs/streams; sim drives via HTTP/gRPC.
* Optional F2/SSF gates via trait (in‑proc) or gRPC (out‑of‑proc).

### 2.3 Repository layout (monorepo)

```
/specs/                  # protobuf + JSON schemas (frozen interfaces)
/docs/                   # invariants, runbooks, dashboards, ADRs
/crates/
  adic-types/            # ids, keys, QpDigits, feature axes
  adic-crypto/           # hashing, signatures (ed25519 or secp256k1)
  adic-math/             # vp, vp_diff, ball_id
  adic-storage/          # RocksDB layouts, snapshots
  adic-mrw/              # parent selection
  adic-consensus/        # C1–C3 checks, attach pipeline, deposits
  adic-finality/         # F1 (k-core) + plug interfaces for F2/SSF
  adic-network/          # QUIC/libp2p gossip, anti-entropy
  adic-node/             # binary, config, API/stream server
/ops/                    # docker, helm, k8s, grafana
/ui-explorer/            # Vue 3 + Vite frontend (Pinia, Vue Router, TailwindCSS)
/adic-sim/               # python sims (uses libadic)
/.github/workflows/      # build, test, fuzz, bench, docker
```

### 2.4 Explorer (Vue) quick map

**Stack:** Vue 3 (Composition API) + Vite + TypeScript; Pinia; Vue Router; TailwindCSS.
**Graph:** Cytoscape.js (via vue‑cytoscape). **Charts:** Apache ECharts (via vue‑echarts).
**Streaming:** **SSE** for tips/finality; optional WebSocket multiplexing.

Routes → components: `/`→`LiveTangle.vue`, `/trace/:id`→`MrwTrace.vue`, `/finality/:id`→`FinalityMonitor.vue`, `/conflicts/:cid`→`ConflictEnergy.vue`, `/params`→`ParamsPanel.vue`.

---

## 3) Protocol & data model

### 3.1 Message schema (protobuf excerpt)

```proto
message AdicMessage {
  bytes id              = 1;      // hash of canonical encoding
  repeated bytes parents= 2;      // exactly d+1 parent ids
  AdicFeatures features = 3;      // Q_p encodings per axis (LSB-first digits)
  AdicMeta meta         = 4;      // ts, axis tags, conflict set id
  bytes proposer_pk     = 5;      // public key
  bytes signature       = 6;      // over fields 1..5
}
message AdicFeatures { repeated AdicAxisPhi phi = 1; }
message AdicAxisPhi { uint32 axis=1; bytes qp_digits=2; uint32 p=3; }
message AdicMeta { uint64 ts=1; map<string,string> axes=2; string conflict=3; }
```

**Convention:** digits **LSB‑first**; ball id at radius `ρ` = first `ρ` digits.

### 3.2 Finality artifact (JSON)

```json
{
  "intent_id": "adic:msg:0x…",
  "gate": "F1",
  "params": {"k":20,"q":3,"D*":12},
  "witness": {"kcore_root":"0x…","depth":14,"diversity_ok":true},
  "validators": [{"pk":"…","sig":"…"}],
  "timestamp": "2025-08-24T12:00:00Z"
}
```

### 3.3 Deposits/receipts

Escrow on accept → Refund on finality → Slash on objective faults.

---

## 4) Algorithms

### 4.1 Admissibility C1–C3 (static floors)

* **C1 (proximity):** `vp(φ_j(x) − φ_j(a_k)) ≥ ρ_j` ∀ axes/parents.
* **C2 (diversity):** ≥ `q` distinct balls at radius `ρ_j` per axis.
* **C3 (floors):** per‑parent `rep(a_k) ≥ r_min`; aggregate `Σ rep(a_k) ≥ R_sum_min` (defaults: `r_min=1.0`, `R_sum_min=4.0`).

### 4.2 MRW parent selection — fully specified

**Per‑axis kernel.** For axis `j`, tip `y`, message `x`:

* Proximity: $\mathrm{prox}_j(x,y) = \min( vp(φ_j(x)-φ_j(y)),\,ρ_j ) / ρ_j \in [0,1]$.
* **Trust:** $\mathrm{trust}(y)=\log(1+\mathrm{rep}(y))$.
  **Rationale:** the log transform **dampens extreme reputation values** to avoid winner‑take‑all centralization while **preserving ordering** and providing **diminishing returns** for very high reputation; this maintains exploration without discarding trust altogether.
* Conflict penalty: `conf(y) ∈ [0,1]` (binary default; optional energy‑scaled variant).

**Weight:** $ w_j(y|x)=\exp\{\lambda\,\mathrm{prox}_j + \beta\,\mathrm{trust} - \mu\,\mathrm{conf}\}$ with defaults `λ=1.0, β=0.5, μ=1.0`.

**Merge & diversity:** sample from each axis distribution, dedup; widen horizon until `d+1` distinct parents satisfy C2.

**Trace:** persist candidates/weights, chosen parents, radius/horizon at decision time.

### 4.3 Finality gate F1 (k‑core)

Windowed forward cone; incremental k‑core; require `depth ≥ D*`; emit artifacts; refund on success.

### 4.4 Conflict energy (telemetry)

Compute signed support `E_t` per conflict id; expose as time series; optional use in penalty scaling.

---

## 5) Storage model (RocksDB)

CFs: `msgs/`, `parents/`, `children/`, `tips/`, `axis_ball_index/{axis,ρ}/`, `kcore_cache/`, `conflicts/`, `conflict_energy/`, `deposits/`, `rep/`.
Snapshots: window snapshots every N minutes; nightly cold backups; restore → anti‑entropy.

---

## 6) Networking & gossip

Transport: **QUIC** or **libp2p** with noise; axis‑aware overlays for diversity; anti‑entropy reconciliation; peer scoring + rate limits + objective‑fault bans.

---

## 7) Node pipeline

```
on_receive(x): verify_signature → quick acyclicity → admissible_C1C3 → escrow → attach_simplex
periodic_finalize(): if final_by_kcore(x): refund; emit_finality_artifact
```

---

## 8) Public APIs & streams

HTTP: `/v1/params`, `/v1/message/:id`, `/v1/finality/:id`, `/v1/conflict/:cid/energy`, `/v1/metrics`.
SSE: `/v1/stream/tips`, `/v1/stream/finality`.
Internal gRPC: `SelectParents`, `FinalizeWindow`.

---

## 9) Feature encoders (canonical Phase‑0 set)

**Interface:** `specs/encoders.json` defines axes, `ρ`, precision, and encoder names.

* `time_epoch` (digits of epoch seconds, base‑p, LSB‑first; precision ≥ ρ+6)
* `topic_hash:blake3` (hash string → base‑p digits)
* `region_code` (ISO code → integer → base‑p digits)

---

## 10) Configuration & flags (excerpt `adic.toml`)

```toml
[c3]
r_min=1.0
R_sum_min=4.0
[mrw]
lambda=1.0
beta=0.5
mu=1.0
conflict_energy_scaled=false
E_cap=10.0
```

(See full file for p2p, consensus, storage, telemetry.)

---

## 11) Observability

Prometheus: MRW candidates/widen/penalty; C1/C2/C3 failures; finality success/latency; axis diversity; conflict energy; deposit states.
Logs: structured JSON with explicit failure reasons and MRW weight snapshots.
Explorer: Live DAG, MRW Trace, Finality monitor, Conflict energy, Diversity, Params.

---

## 12) Testing & verification

Unit/property tests (vp/vp\_diff, ball stability, C1/C2/C3); fuzz (malformed payloads, sig failures, conflict ids); integration (multi‑node churn/partition/rejoin; snapshot restore); sims (Poisson arrivals; conflicts; sybil rings; parameter sweeps).

---

## 13) CI/CD & releases

Pipeline: lint → unit/property → fuzz (time‑boxed) → integration (docker) → build containers → SBOM → sign.
Artifacts: static Linux binary; Docker images (`adic-node`, `adic-explorer-api`, `ui-explorer`).
Versioning: `v0.y.z-testnet` with reproducible notes.

---

## 14) Deployment (Dev/Stage/Public Testnet)

Docker (non‑root, read‑only FS, tmpfs caches).
Kubernetes (Helm): StatefulSet (PVC for RocksDB), PDB, probes; ConfigMap `adic.toml`, Secret keys; Services for API/metrics.
Topology: 3–5 bootstrap seeds; explorer reads from replicas.
SLOs: Finality p50 ≤ 12s, p95 ≤ 30s; MRW select p99 ≤ 50ms (nominal tips).

---

## 15) Genesis & bootstrapping

Publish genesis hash, default params, bootstrap peers; `bootstrap.sh` to init/start; signed launch notes with SHAs + SBOM links.

---

## 16) Upgrade & compatibility

Header schema version; forward‑compatible proto fields; graceful drain (stop gossip → finalize window → snapshot → shutdown).

---

## 17) Runbooks (abridged)

Node won’t start (storage) → backup → restore snapshot → `--reindex` → verify anti‑entropy/explorer.
High C1/C2/C3 failures → check encoders/ρ/precision; MRW widen; peer diversity; floors.
Finality latency spike → k‑core cache/window; tune `D*`/`k`; check conflicts/DB stalls.
Deposit anomalies → reconcile counters; audit objective faults; confirm slashing.

---

## 18) Risks & mitigations

Starvation/fairness → MRW entropy alarms; horizon widening; tune `λ,μ`.
Gossip view skew → anti‑entropy cadence; axis‑aware peers; minimum peer diversity.
DB hot spots → CF separation; batched writes; compaction strategy; rate‑limit attaches.
Adversarial spam → deposit sizing; ingress caps; objective slashing; static floors.
Complexity creep → frozen `/specs/`; ADRs for changes.

---

## 19) Milestones & rollout plan

**Formal checkpoint:** **Parameter Calibration Review (end of T‑3 → early T‑4)**

* Inputs: sim sweeps for `(λ, β, μ, r_min, R_sum_min, ρ, q, k, D*, Δ)` + soak‑test telemetry (finality latency, MRW widen rate, diversity, conflict energy drift).
* Outputs: **frozen Phase‑0 params**, calibration report (figures + rationale), sign‑off by Protocol, Research, and SRE; config stamped in release notes.

---

## 20) Definition of Done (Phase‑0)

Consensus admit→attach→finalize (F1) under defaults; artifacts emitted & verifiable; observability proves invariants (diversity, MRW traces, k‑core growth, conflict drift); CI/CD green; ops runbooks & snapshots; security: objective slashing, rate limits, deposits, signed binaries.

---
