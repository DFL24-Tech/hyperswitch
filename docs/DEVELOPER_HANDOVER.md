# Hyperswitch Developer Handover Guide

> **Audience:** New developers joining the project who need to understand the architecture, codebase layout, and how to extend it.

---

## Table of Contents

- [Hyperswitch Developer Handover Guide](#hyperswitch-developer-handover-guide)
  - [Table of Contents](#table-of-contents)
  - [1. What Is Hyperswitch?](#1-what-is-hyperswitch)
  - [2. High-Level Architecture](#2-high-level-architecture)
    - [Runtime Processes](#runtime-processes)
  - [3. Repository Layout](#3-repository-layout)
  - [4. Data Flow: A Payment Request End-to-End](#4-data-flow-a-payment-request-end-to-end)
    - [Key Abstractions](#key-abstractions)
  - [5. Crates In-Depth](#5-crates-in-depth)
    - [5.1 `router` — The Main Application](#51-router--the-main-application)
      - [Internal Module Map](#internal-module-map)
      - [The Operation Pattern](#the-operation-pattern)
      - [AppState vs SessionState](#appstate-vs-sessionstate)
      - [Routing Integration](#routing-integration)
      - [Webhooks](#webhooks)
    - [5.2 `hyperswitch_interfaces` — Connector Contracts](#52-hyperswitch_interfaces--connector-contracts)
      - [Key Traits](#key-traits)
    - [5.3 `hyperswitch_connectors` — Connector Implementations](#53-hyperswitch_connectors--connector-implementations)
      - [How a Connector Processes a Payment](#how-a-connector-processes-a-payment)
      - [Default Implementations](#default-implementations)
    - [5.4 `hyperswitch_domain_models` — Core Domain Types](#54-hyperswitch_domain_models--core-domain-types)
      - [Key Modules](#key-modules)
      - [RouterData](#routerdata)
    - [5.5 `api_models` — HTTP API Request / Response Shapes](#55-api_models--http-api-request--response-shapes)
      - [Structure](#structure)
    - [5.6 `common_utils` / `common_enums` / `common_types`](#56-common_utils--common_enums--common_types)
    - [5.7 `diesel_models` — Database ORM Layer](#57-diesel_models--database-orm-layer)
    - [5.8 `storage_impl` — Storage Abstraction](#58-storage_impl--storage-abstraction)
    - [5.9 `redis_interface` — Redis Client](#59-redis_interface--redis-client)
    - [5.10 `euclid` — Routing DSL \& Static Analyser](#510-euclid--routing-dsl--static-analyser)
      - [Sub-modules](#sub-modules)
    - [5.11 `kgraph_utils` — Constraint Graph for Routing](#511-kgraph_utils--constraint-graph-for-routing)
    - [5.12 `analytics` — Observability \& Metrics](#512-analytics--observability--metrics)
      - [Data Domains](#data-domains)
    - [5.13 `scheduler` — Process Tracker](#513-scheduler--process-tracker)
    - [5.14 `drainer` — KV to DB Sync](#514-drainer--kv-to-db-sync)
    - [5.15 `router_env` — Logging \& Telemetry](#515-router_env--logging--telemetry)
    - [5.16 `router_derive` — Proc Macros](#516-router_derive--proc-macros)
    - [5.17 `payment_methods` — Vault \& PM Management](#517-payment_methods--vault--pm-management)
    - [5.18 `external_services` — Third-Party Integrations](#518-external_services--third-party-integrations)
    - [5.19 Other Smaller Crates](#519-other-smaller-crates)
  - [6. Configuration System](#6-configuration-system)
  - [7. Feature Flags](#7-feature-flags)
  - [8. Database \& Migrations](#8-database--migrations)
    - [Running Migrations](#running-migrations)
  - [9. How to Add a New Connector](#9-how-to-add-a-new-connector)
    - [Step 1: Scaffold](#step-1-scaffold)
    - [Step 2: Implement `transformers.rs`](#step-2-implement-transformersrs)
    - [Step 3: Implement `ConnectorCommon` and `ConnectorIntegration`](#step-3-implement-connectorcommon-and-connectorintegration)
    - [Step 4: Register the Connector](#step-4-register-the-connector)
    - [Step 5: Test](#step-5-test)
  - [10. How to Add a New API Endpoint](#10-how-to-add-a-new-api-endpoint)
    - [Step 1: Define request/response types in `api_models`](#step-1-define-requestresponse-types-in-api_models)
    - [Step 2: Add domain logic in `router/src/core/my_feature.rs`](#step-2-add-domain-logic-in-routersrccoremy_featurers)
    - [Step 3: Add route handler in `router/src/routes/my_feature.rs`](#step-3-add-route-handler-in-routersrcroutesmy_featurers)
    - [Step 4: Register in `routes/app.rs`](#step-4-register-in-routesapprs)
  - [11. Testing](#11-testing)
  - [12. Running Locally](#12-running-locally)
    - [Docker (Recommended)](#docker-recommended)
    - [From Source](#from-source)
    - [Useful `just` Commands](#useful-just-commands)
    - [Environment Variable Overrides](#environment-variable-overrides)

---

## 1. What Is Hyperswitch?

Hyperswitch is an **open-source, composable payments infrastructure** built in Rust. It acts as a unified payment router that sits between your application and dozens of Payment Service Providers (PSPs). It is often described as "Linux for Payments".

**Core capabilities:**

| Module | What it does |
|---|---|
| **Payment Routing** | Rule-based and ML-driven routing to the best PSP per transaction |
| **Revenue Recovery** | Automatic retries for failed payments |
| **Vault** | PCI-compliant storage of cards, tokens, wallets |
| **Cost Observability** | Analytics dashboards for payment cost analysis |
| **Alternate Payment Methods** | Drop-in widgets for wallets, BNPL, bank pay |
| **Reconciliation** | Automated 2-way/3-way reconciliation |

---

## 2. High-Level Architecture

```mermaid
graph TB
    Client["Client / Merchant App"]
    ControlCenter["Control Center (Dashboard UI)"]

    subgraph "Hyperswitch Core"
        Router["router<br>(Actix-Web HTTP server)"]
        Scheduler["scheduler<br>(Process Tracker)"]
        Drainer["drainer<br>(KV → DB sync)"]
        Analytics["analytics<br>(ClickHouse / OpenSearch)"]
    end

    subgraph "Storage"
        Postgres["PostgreSQL<br>(primary store)"]
        Redis["Redis<br>(cache + KV store + pub/sub)"]
    end

    subgraph "External"
        Connectors["PSP Connectors<br>(Stripe, Adyen, ~144 others)"]
        AWS["AWS KMS / S3"]
        Locker["Card Locker / Vault"]
        GrpcServices["gRPC Services<br>(Dynamic Routing, Revenue Recovery, UCS)"]
    end

    Client -->|REST API| Router
    ControlCenter -->|REST API| Router
    Router -->|DB reads/writes| Postgres
    Router -->|KV reads/writes| Redis
    Router -->|HTTPS| Connectors
    Router -->|gRPC| GrpcServices
    Router -->|Secrets| AWS
    Router -->|Card storage| Locker
    Scheduler -->|Polls process_tracker| Postgres
    Scheduler -->|Triggers workflows| Router
    Drainer -->|Reads Redis streams| Redis
    Drainer -->|Bulk writes| Postgres
    Analytics -->|Query events| ClickHouse
```

### Runtime Processes

The workspace builds **two binary executables**:

| Binary | Entry point | Role |
|---|---|---|
| `router` | `crates/router/src/bin/router.rs` | Main HTTP API server |
| `scheduler` | `crates/router/src/bin/scheduler.rs` | Background job/workflow runner |

The `drainer` crate is also a standalone binary that syncs the Redis KV store back to PostgreSQL.

---

## 3. Repository Layout

```
hyperswitch/
├── crates/                  # All Rust code (Cargo workspace members)
├── config/                  # TOML config files per environment
├── migrations/              # PostgreSQL Diesel migrations (~462 total)
├── v2_migrations/           # Migrations for API v2
├── docker/                  # Docker build assets
├── docker-compose*.yml      # Compose files for different profiles
├── scripts/                 # Setup and utility scripts
├── postman/                 # Postman test collections
├── cypress-tests/           # End-to-end browser tests (v1)
├── cypress-tests-v2/        # End-to-end browser tests (v2)
├── docs/                    # Documentation and architecture diagrams
├── api-reference/           # OpenAPI/Smithy API specs
├── connector-template/      # cargo-generate template for new connectors
├── loadtest/                # Load testing scripts
├── monitoring/              # Grafana/Prometheus dashboards
├── proto/                   # Protobuf definitions for gRPC services
└── smithy/                  # Smithy model definitions
```

---

## 4. Data Flow: A Payment Request End-to-End

This traces a `POST /payments` (create + confirm) through the system.

```mermaid
sequenceDiagram
    participant Client
    participant Router as router (Actix)
    participant Core as core::payments
    participant Routing as Euclid Routing Engine
    participant Connector as hyperswitch_connectors
    participant PSP as External PSP API
    participant DB as PostgreSQL
    participant KV as Redis

    Client->>Router: POST /payments {amount, currency, pm_data}
    Router->>Core: PaymentCreate operation
    Core->>DB: Create payment_intent + payment_attempt
    Core->>KV: Cache payment state (KV store)

    Client->>Router: POST /payments/:id/confirm
    Router->>Core: PaymentConfirm operation
    Core->>Core: Validate request, build PaymentData
    Core->>Routing: Select connector (Euclid rules + dynamic routing)
    Routing-->>Core: ConnectorData (e.g. Adyen)
    Core->>Connector: construct_router_data()
    Connector->>Connector: transformers::try_from (Hyperswitch → PSP format)
    Connector->>PSP: HTTPS request
    PSP-->>Connector: Response
    Connector->>Connector: transformers::try_from (PSP → Hyperswitch format)
    Connector-->>Core: RouterData with response
    Core->>DB: Update attempt status
    Core->>KV: Update cache
    Core-->>Router: ApplicationResponse
    Router-->>Client: JSON response
```

### Key Abstractions

| Concept | What it represents |
|---|---|
| `PaymentData<F>` | All mutable state for a payment during processing (intent, attempt, address, etc.) |
| `RouterData<Flow, Req, Resp>` | The typed envelope passed to/from connectors, parameterised on the payment flow type |
| `ConnectorData` | Which connector was selected + its auth config |
| `Operation` trait | Each payment lifecycle step (Create, Confirm, Capture, etc.) implements `GetTracker + Domain + UpdateTracker` |

---

## 5. Crates In-Depth

### 5.1 `router` — The Main Application

**Location:** `crates/router/`  
**Binary:** `router` (Actix-Web HTTP server)

This is the largest crate and the heart of the system. It wires together every other crate.

#### Internal Module Map

```
router/src/
├── bin/
│   ├── router.rs          # main() – boots Actix server
│   └── scheduler.rs       # main() – boots process tracker
├── routes/
│   ├── app.rs             # AppState, SessionState, Actix Scope builders
│   ├── payments.rs        # Route handlers for /payments
│   ├── refunds.rs         # Route handlers for /refunds
│   ├── admin.rs           # Merchant/profile management
│   ├── webhooks.rs        # Inbound webhook ingestion
│   └── … (one file per domain)
├── core/
│   ├── payments/          # Payment lifecycle orchestration
│   │   ├── operations/    # One file per operation (create, confirm, capture…)
│   │   ├── flows/         # Per-flow connector call wrappers
│   │   ├── gateway/       # Low-level connector dispatch
│   │   ├── routing/       # Euclid + dynamic routing integration
│   │   ├── helpers.rs     # Shared helpers
│   │   └── transformers.rs
│   ├── refunds/           # Refund orchestration
│   ├── webhooks/          # Inbound + outbound webhook processing
│   ├── routing.rs         # Routing algorithm CRUD + orchestration
│   ├── admin.rs           # Merchant account management
│   └── … (one module per domain)
├── db/                    # Trait implementations for each DB entity
├── configs/
│   └── settings.rs        # All `Settings<S>` structs (deserialized from TOML)
├── services/              # API call execution, authentication, encryption helpers
├── workflows/             # Process tracker workflow definitions
├── events/                # Audit event emission
├── middleware.rs          # Request logging, auth middleware
└── types/                 # Re-exports and type aliases used throughout router
```

#### The Operation Pattern

Every payment lifecycle step is modelled as a struct that implements four traits:

```mermaid
flowchart LR
    VR[ValidateRequest] --> GT[GetTracker<br>load state from DB/KV]
    GT --> DM[Domain<br>business logic]
    DM --> UT[UpdateTracker<br>write changes back]
    UT --> CC[CallConnector<br>dispatch to PSP]
    CC --> PUT[PostUpdateTracker<br>finalize]
```

Example operations and what they do:

| File | Payment action |
|---|---|
| `payment_create.rs` | Creates `payment_intent` + `payment_attempt` records |
| `payment_confirm.rs` | Validates PM data, calls routing, dispatches to connector |
| `payment_capture.rs` | Captures a previously authorized payment |
| `payment_cancel.rs` | Voids/cancels an authorized payment |
| `payment_sync.rs` | Syncs status from the PSP |
| `payment_response.rs` | Handles the connector response and updates state |

#### AppState vs SessionState

- **`AppState`** – singleton, lives for the process lifetime. Holds DB pool, Redis pool, config, gRPC clients.
- **`SessionState`** – per-request, scoped to a tenant. Wraps `AppState` plus the tenant ID and tenant-specific config.

#### Routing Integration

The `core/payments/routing.rs` module integrates Euclid:
1. Loads the merchant's routing algorithm from Redis cache (falls back to DB).
2. Evaluates the Euclid DSL rules against the current `PaymentData`.
3. Optionally overlays dynamic/ML-based routing (success-rate, elimination, contract-based).
4. Returns an ordered list of `ConnectorData` choices.

#### Webhooks

- **Inbound** (`core/webhooks/incoming.rs`): Receives events from PSPs, verifies signatures via the connector's `IncomingWebhook` trait impl, maps to Hyperswitch events, and updates payment/refund state.
- **Outbound** (`core/webhooks/outgoing.rs`): Sends Hyperswitch events to merchant-configured webhook endpoints, with retries via the process tracker.

---

### 5.2 `hyperswitch_interfaces` — Connector Contracts

**Location:** `crates/hyperswitch_interfaces/`

Defines the **trait interfaces** every connector must implement. No concrete implementations live here — it is a pure contract crate.

#### Key Traits

| Trait | Purpose |
|---|---|
| `Connector` | Marker super-trait; all connectors implement this |
| `ConnectorCommon` | Base info: base URL, auth type, error parsing |
| `ConnectorValidation` | Pre-flight checks (supported currencies, PM types) |
| `ConnectorIntegration<Flow, Req, Resp>` | The core v1 trait — `get_headers`, `get_url`, `get_request_body`, `handle_response` |
| `ConnectorIntegrationV2<Flow, FlowData, Req, Resp>` | V2 variant for the new connector architecture |
| `ConnectorSpecifications` | Describes which payment methods/flows the connector supports |
| `IncomingWebhook` | Parses and verifies inbound webhook events |
| `ConnectorAccessToken` | OAuth access token acquisition |
| `ConnectorCommonExt` | Helper extension combining `ConnectorCommon + ConnectorIntegration` |

```mermaid
classDiagram
    class ConnectorCommon {
        +id() str
        +get_currency_unit() CurrencyUnit
        +common_get_content_type() str
        +base_url(connectors) str
        +get_auth_header(auth_type) Vec~Header~
        +build_error_response(res) ErrorResponse
    }
    class ConnectorIntegration~Flow Req Resp~ {
        +get_headers(router_data, connectors) Vec~Header~
        +get_url(router_data, connectors) str
        +get_request_body(router_data, connectors) RequestContent
        +build_request(router_data, connectors) Request
        +handle_response(router_data, event_builder, res) RouterData
        +get_error_response(res, event_builder) ErrorResponse
    }
    class IncomingWebhook {
        +get_webhook_source_verification_merchant_secret()
        +verify_webhook_source()
        +get_webhook_event_type() IncomingWebhookEvent
        +get_webhook_resource_object() Value
    }
    ConnectorCommon <|-- ConnectorIntegration
```

The `RouterDataConversion` trait bridges v1 `RouterData` and v2 `RouterDataV2`, enabling connectors to be progressively migrated.

---

### 5.3 `hyperswitch_connectors` — Connector Implementations

**Location:** `crates/hyperswitch_connectors/`  
**Contains:** ~144 connector directories

Each connector lives in `src/connectors/<name>/` and has two files:
- `<name>.rs` — Implements `ConnectorCommon`, `ConnectorIntegration` for each flow, `IncomingWebhook`
- `<name>/transformers.rs` — Rust types matching the PSP's JSON API + `TryFrom` implementations

#### How a Connector Processes a Payment

```mermaid
flowchart LR
    HS[RouterData<br>Hyperswitch types] -->|transformers::try_from| PSPReq[PSP Request<br>JSON struct]
    PSPReq -->|serialize| HTTP[HTTP Body]
    HTTP -->|HTTPS| PSP[PSP API]
    PSP -->|response| ParsedResp[PSP Response<br>JSON struct]
    ParsedResp -->|transformers::try_from| HS2[RouterData<br>with response]
```

**Example: Adyen**
- `connectors/adyen.rs` implements `ConnectorIntegration<Authorize, PaymentsAuthorizeData, PaymentsResponseData>`, `ConnectorIntegration<Capture, …>`, dispute handling, webhook parsing, etc.
- `connectors/adyen/transformers.rs` defines `AdyenPaymentRequest`, `AdyenPaymentResponse`, and the `TryFrom` conversions.

#### Default Implementations

`default_implementations.rs` and `default_implementations_v2.rs` provide blanket no-op implementations of optional traits, so connectors only override what they actually support.

---

### 5.4 `hyperswitch_domain_models` — Core Domain Types

**Location:** `crates/hyperswitch_domain_models/`

The **canonical data model** for every business entity. All other crates depend on this; it depends on very little. This is where the "source of truth" types live.

#### Key Modules

| Module | What it models |
|---|---|
| `payments/payment_intent.rs` | `PaymentIntent` — top-level payment record |
| `payments/payment_attempt.rs` | `PaymentAttempt` — individual connector attempt |
| `router_data.rs` | `RouterData<Flow, Req, Resp>` — the connector envelope |
| `router_request_types/` | One struct per flow request (e.g., `PaymentsAuthorizeData`) |
| `router_response_types/` | One struct per flow response (e.g., `PaymentsResponseData`) |
| `router_flow_types/` | Zero-sized marker types for each flow (e.g., `Authorize`, `Capture`) |
| `merchant_account.rs` | `MerchantAccount` — merchant config |
| `merchant_connector_account.rs` | `MerchantConnectorAccount` — per-connector credentials |
| `business_profile.rs` | `BusinessProfile` — sub-account / profile |
| `payment_method_data.rs` | `PaymentMethodData` — card, wallet, bank, etc. |
| `address.rs` | `Address` / `AddressDetails` |
| `refunds.rs` | `Refund` domain type |
| `mandates.rs` | Mandate data |
| `routing.rs` | `RoutingData`, `PreRoutingConnectorChoice` |

#### RouterData

```rust
pub struct RouterData<Flow, Request, Response> {
    pub flow: PhantomData<Flow>,      // zero-sized marker
    pub merchant_id: MerchantId,
    pub connector: String,
    pub payment_id: String,
    pub status: AttemptStatus,
    pub connector_auth_type: ConnectorAuthType,
    pub request: Request,             // flow-specific request data
    pub response: Result<Response, ErrorResponse>, // flow-specific response
    // ... billing address, auth token, metadata, etc.
}
```

`Flow` is a phantom type — it encodes at the type level which payment flow is in progress, enabling the compiler to enforce correct trait implementations.

---

### 5.5 `api_models` — HTTP API Request / Response Shapes

**Location:** `crates/api_models/`

Contains the **public API types** — exactly what merchants serialize/deserialize over HTTP. These are distinct from the internal domain models.

#### Structure

```
api_models/src/
├── payments.rs            # PaymentsRequest, PaymentsResponse, etc.
├── refunds.rs             # RefundsRequest, RefundsResponse
├── admin.rs               # Merchant, profile, connector management APIs
├── routing.rs             # Routing algorithm API types
├── webhooks.rs            # Webhook event types
├── analytics/             # Analytics query/response types
├── customers.rs
├── disputes.rs
├── mandates.rs
├── payouts.rs
└── … (one file per API domain)
```

The separation between `api_models` (external-facing) and `hyperswitch_domain_models` (internal) is intentional: it allows the internal representation to evolve without breaking the public API contract, and vice versa. Conversion happens via `ForeignFrom`/`ForeignTryFrom` traits.

---

### 5.6 `common_utils` / `common_enums` / `common_types`

**`common_utils`** (`crates/common_utils/`) — Foundational utilities used by every crate:
- `id_type` — newtype wrappers for IDs (`MerchantId`, `PaymentId`, `CustomerId`, etc.)
- `crypto` — hashing, encryption helpers
- `pii` — PII-safe types (`Secret<T>`, email, phone masking)
- `ext_traits` — extension traits for `Option`, `Result`, `str`, etc.
- `date_time` — date/time formatting utilities
- `request` — `Request` and `RequestBuilder` for outbound HTTP
- `validation` — common field validators

**`common_enums`** (`crates/common_enums/`) — All shared enumerations: `PaymentMethod`, `Currency`, `AttemptStatus`, `ConnectorType`, etc.

**`common_types`** (`crates/common_types/`) — Simple value types used across crates (e.g., `MinorUnit` for amounts, `CustomerAcceptance`).

---

### 5.7 `diesel_models` — Database ORM Layer

**Location:** `crates/diesel_models/`

Contains the **Diesel ORM models** — one module per database table. These types map 1:1 to Postgres table rows.

Each module typically has:
- A `<Entity>` struct (read from DB)
- A `<Entity>New` struct (insert)
- An `<Entity>Update` struct (update)
- Query helper functions using Diesel's DSL

Also contains `kv.rs` with types for the Redis KV store (used by the drainer).

Key entities: `payment_intent`, `payment_attempt`, `merchant_account`, `merchant_connector_account`, `refund`, `dispute`, `mandate`, `routing_algorithm`, `process_tracker`, `events`, `role`, `user`, …

---

### 5.8 `storage_impl` — Storage Abstraction

**Location:** `crates/storage_impl/`

Implements the **`StorageInterface` trait** (defined in `router/src/db/`) for:

1. **`RouterStore`** — Production implementation: writes go to Redis KV (fast path), the drainer periodically flushes to Postgres. Reads check KV first, fall back to Postgres.
2. **`KVRouterStore`** — Thin wrapper enforcing KV-first reads.
3. **`MockDb`** — In-memory implementation for tests.

This indirection means business logic in `router/core/` never talks directly to Diesel or Redis — it calls trait methods on `StorageInterface`, keeping it testable.

---

### 5.9 `redis_interface` — Redis Client

**Location:** `crates/redis_interface/`

A thin wrapper around the `fred` Redis client library. Provides:
- `RedisConnectionPool` — connection pooling
- `commands.rs` — typed wrappers for all used Redis commands (GET, SET, HSET, ZADD, XADD streams, pub/sub, etc.)
- TTL management and key prefixing
- Pub/sub subscriber and publisher clients (used for cache invalidation)

---

### 5.10 `euclid` — Routing DSL & Static Analyser

**Location:** `crates/euclid/`

Euclid is a **custom domain-specific language and evaluation engine** for writing payment routing rules. Merchants define rules in the Control Center ("route to Stripe if currency = USD and amount > 100"), which get compiled into a form Euclid evaluates at runtime.

```mermaid
graph LR
    MerchantRule["Merchant rule<br>(JSON / UI)"]
    AST["AST<br>frontend::ast"]
    DIR["Decision IR<br>frontend::dir (Discriminant IR)"]
    Backend["EuclidBackend<br>(evaluator)"]
    Result["ConnectorList<br>(ordered)"]

    MerchantRule --> AST
    AST --> DIR
    DIR --> Backend
    Backend --> Result
```

#### Sub-modules

| Module | Purpose |
|---|---|
| `frontend/ast.rs` | Abstract syntax tree for rule definitions |
| `frontend/dir.rs` | Discriminant IR — an intermediate representation optimised for evaluation |
| `frontend/vir.rs` | Value IR |
| `backend/` | The evaluator that walks the IR given `PaymentData` inputs |
| `dssa/` | **Domain Specific Static Analyser** — detects conflicting/unreachable rules at rule-save time |
| `dssa/graph.rs` | Constraint graph (cgraph) representation |
| `dssa/analyzer.rs` | Static analysis passes |
| `enums.rs` | All dimensions Euclid can evaluate (currency, amount, pm_type, country, etc.) |

The `dssa` (static analyser) runs when a merchant saves a routing rule and reports conflicts (e.g., "Rule A and Rule B can never both match") and dead code.

---

### 5.11 `kgraph_utils` — Constraint Graph for Routing

**Location:** `crates/kgraph_utils/`

Builds a **constraint graph** that represents which connectors support which combination of `(payment_method, pm_type, currency, country)`. Used by the routing engine to filter eligible connectors before applying Euclid rules.

| Module | Purpose |
|---|---|
| `mca.rs` | Builds the graph from `MerchantConnectorAccount` configuration |
| `transformers.rs` | Converts MCA data into Euclid `dir` values for graph nodes |
| `types.rs` | `CountryCurrencyFilter` and related types |

---

### 5.12 `analytics` — Observability & Metrics

**Location:** `crates/analytics/`  

Provides the analytics API endpoints (used by the Control Center). Queries are executed against ClickHouse (OLAP) or OpenSearch.

#### Data Domains

| Module | Metrics tracked |
|---|---|
| `payments/` | Payment success rate, volume, latency by connector, PM, currency |
| `payment_intents/` | Intent-level funnel metrics |
| `refunds/` | Refund rate, processing time |
| `disputes/` | Dispute rate by connector |
| `api_event/` | API endpoint latency and error rates |
| `sdk_events/` | Checkout SDK events (funnel, drop-off) |
| `auth_events/` | 3DS authentication metrics |
| `routing_events/` | Routing decision audit logs |
| `outgoing_webhook_event/` | Webhook delivery stats |

Each domain follows the same pattern: `metrics.rs` (metric definitions), `dimensions.rs` (group-by dimensions), `filters.rs`, `core.rs` (query logic).

---

### 5.13 `scheduler` — Process Tracker

**Location:** `crates/scheduler/`

A **background task queue** backed by Postgres (`process_tracker` table). The router creates tasks in this table (e.g., "retry this outbound webhook in 5 minutes"), and the scheduler binary polls and dispatches them.

```mermaid
flowchart TD
    PT[process_tracker table<br>Pending tasks]
    Consumer[consumer.rs<br>Polls for pending tasks]
    Workflow[Workflow trait<br>e.g. outgoing_webhook_retry]
    Execute[Execute workflow logic]
    Reschedule{Retry needed?}
    Done[Mark task complete]

    PT --> Consumer
    Consumer --> Workflow
    Workflow --> Execute
    Execute --> Reschedule
    Reschedule -->|Yes| PT
    Reschedule -->|No| Done
```

**Workflows** defined in `router/src/workflows/`:
- `payment_sync.rs` — Poll PSP for payment status updates
- `refund_router.rs` — Retry refund submission
- `outgoing_webhook_retry.rs` — Re-deliver failed webhooks
- `revenue_recovery.rs` — Passive churn retry logic
- `invoice_sync.rs` — Billing invoice status sync
- `payment_method_status_update.rs` — Update saved PM status

---

### 5.14 `drainer` — KV to DB Sync

**Location:** `crates/drainer/`

The drainer reads from **Redis Streams** (the append log that `storage_impl` writes to) and bulk-upserts the data into PostgreSQL. This decouples write latency (fast Redis writes) from Postgres durability.

Key files:
- `stream.rs` — reads Redis stream entries
- `handler.rs` — processes each entry and routes to the correct Diesel insert/update
- `query.rs` — Diesel queries used for bulk upserts

The drainer is a separate binary that should run continuously alongside the router.

---

### 5.15 `router_env` — Logging & Telemetry

**Location:** `crates/router_env/`

Shared logging/tracing infrastructure:
- Wraps `tracing` and `tracing-actix-web` for structured, context-aware logging
- `logger.rs` — initialisation of the tracing subscriber (console, file, OpenTelemetry)
- `metrics.rs` — OpenTelemetry metrics setup
- `request_id.rs` — Request-ID propagation middleware
- `root_span.rs` — Custom Actix root span builder
- `vergen.rs` — Compile-time git metadata (version, commit hash)

Usage across the codebase: `router_env::{instrument, tracing}` decorators on async functions.

---

### 5.16 `router_derive` — Proc Macros

**Location:** `crates/router_derive/`

Custom derive macros to reduce boilerplate:

| Macro | What it generates |
|---|---|
| `#[derive(PaymentOperation)]` | Implements `Operation` trait boilerplate for payment operation structs |
| `#[derive(DebugAsDisplay)]` | `Display` from `Debug` |
| `#[derive(ApiEventMetric)]` | Metric emission boilerplate |
| `#[diesel(…)]` | Diesel enum/table mapping helpers |

---

### 5.17 `payment_methods` — Vault & PM Management

**Location:** `crates/payment_methods/`

Manages stored payment methods (vault interaction):
- `core/` — Business logic: save card, list saved PMs, delete PM
- `client.rs` — HTTP client to the external card locker service
- `controller.rs` — Request orchestration
- `helpers.rs` — Tokenisation, locker interaction helpers
- `configs/` — Payment method eligibility config (which PM types are available per connector)

The locker is an external microservice (can be mocked via `mock_locker = true` in config).

---

### 5.18 `external_services` — Third-Party Integrations

**Location:** `crates/external_services/`

Abstractions over external dependencies (feature-gated):

| Module | Service |
|---|---|
| `aws_kms` | AWS KMS for secret decryption |
| `email/` | SES, SMTP, no-op email backends |
| `file_storage/` | S3 / local file storage |
| `grpc_client/` | gRPC clients: dynamic routing, revenue recovery, UCS (Unified Connector Service) |
| `hashicorp_vault` | HashiCorp Vault for secrets |
| `superposition` | Feature flag / remote config service |
| `crm` | CRM webhook integrations |

---

### 5.19 Other Smaller Crates

| Crate | Purpose |
|---|---|
| `common_enums` | Shared enumerations (all payment-domain enums) |
| `common_types` | Shared primitive types |
| `cards` | Card number utilities, BIN lookup, network token types |
| `currency_conversion` | Exchange rate utilities |
| `pm_auth` | Payment method authentication (open banking, Plaid) |
| `openapi` | OpenAPI spec generation |
| `payment_link` | Hosted payment link rendering |
| `euclid_macros` | Macros for Euclid rule definition |
| `euclid_wasm` | WASM build of Euclid (for browser-side rule preview) |
| `hyperswitch_constraint_graph` | Generic constraint graph library used by Euclid |
| `injector` | Dependency injection helpers |
| `events` | Kafka/audit event emission |
| `subscriptions` | Subscription/recurring billing logic |
| `test_utils` | Shared test utilities |
| `hsdev` | Developer tooling helpers |

---

## 6. Configuration System

All runtime configuration is in TOML files under `config/`.

| File | Environment |
|---|---|
| `config/development.toml` | Local development |
| `config/docker_compose.toml` | Docker Compose |
| `config/docker_compose.local.toml` | Docker local |
| `config/config.example.toml` | Annotated reference |

The `Settings<S>` struct (in `router/src/configs/settings.rs`) is deserialized at startup using the `config` crate. It supports layered overrides via environment variables (prefixed `ROUTER__`).

Key configuration sections:

| Section | Controls |
|---|---|
| `[server]` | Host, port, request body limits |
| `[master_database]` / `[replica_database]` | Postgres connection |
| `[redis]` | Redis connection, TTLs |
| `[secrets]` | Admin API key, JWT secret, master encryption key |
| `[locker]` | Card vault host, mock mode |
| `[connectors.*]` | Connector base URLs |
| `[log]` | Console/file/telemetry log levels |
| `[key_manager]` | Key manager service config |
| `[forex_api]` | Currency conversion API |

---

## 7. Feature Flags

The `router` crate uses Cargo feature flags extensively to compile in/out functionality:

| Feature | What it enables |
|---|---|
| `v1` | API v1 routes and logic (default) |
| `v2` | API v2 routes (new architecture) |
| `oltp` | Online transaction processing routes |
| `olap` | Analytics / reporting routes |
| `kv_store` | Redis KV fast-path for storage |
| `payouts` | Payout disbursement flows |
| `frm` | Fraud & Risk Management |
| `retry` | Payment retry logic |
| `email` | Email notification support |
| `tls` | TLS termination |
| `dynamic_routing` | ML-based dynamic connector routing |
| `revenue_recovery` | Passive churn recovery |
| `dummy_connector` | Fake connector for testing |

Feature sets are combined: the default build enables `v1 + kv_store + oltp + olap + payouts + frm + retry + tls`.

---

## 8. Database & Migrations

- **ORM:** Diesel  
- **Config file:** `diesel.toml` (v1), `diesel_v2.toml` (v2)  
- **Migrations location:** `migrations/` (~462 migrations), `v2_migrations/`

### Running Migrations

```bash
# Apply all pending migrations
diesel migration run --database-url postgres://db_user:db_pass@localhost/hyperswitch_db

# Or with just:
just migrate
```

Each migration folder contains `up.sql` and `down.sql`.

---

## 9. How to Add a New Connector

See `add_connector.md` in the repo root for the full official guide. Summary:

### Step 1: Scaffold

```bash
cargo generate --path ./connector-template --name <connector_name>
# Files are created in crates/hyperswitch_connectors/src/connectors/<name>/
```

### Step 2: Implement `transformers.rs`

Define Rust structs matching the PSP's request/response JSON, and implement `TryFrom` conversions:

```rust
// PSP request type
struct MyConnectorPaymentRequest { amount: i64, currency: String, … }

// PSP → Hyperswitch conversion
impl TryFrom<&PaymentsAuthorizeRouterData> for MyConnectorPaymentRequest { … }

// Hyperswitch ← PSP conversion
impl TryFrom<ResponseRouterData<Authorize, MyConnectorPaymentResponse, …>> for RouterData<…> { … }
```

### Step 3: Implement `ConnectorCommon` and `ConnectorIntegration`

In `<name>.rs`:
```rust
impl ConnectorCommon for MyConnector {
    fn id(&self) -> &'static str { "myconnector" }
    fn base_url(&self, connectors: &Connectors) -> &str { &connectors.myconnector.base_url }
    fn build_error_response(&self, res: Response, …) -> ErrorResponse { … }
}

impl ConnectorIntegration<Authorize, PaymentsAuthorizeData, PaymentsResponseData> for MyConnector {
    fn get_url(&self, …) -> String { … }
    fn get_headers(&self, …) -> Vec<(String, Maskable<String>)> { … }
    fn get_request_body(&self, …) -> RequestContent { … }
    fn handle_response(&self, data, event_builder, res) -> RouterData<…> { … }
}
```

### Step 4: Register the Connector

1. Add to `crates/hyperswitch_connectors/src/connectors.rs`
2. Add to `crates/connector_configs/` (Control Center config)
3. Add base URL to `config/config.example.toml`
4. Add to `common_enums` connector enum

### Step 5: Test

```bash
cargo test -p hyperswitch_connectors -- myconnector
# Integration tests in crates/router/tests/connectors/<name>.rs
```

---

## 10. How to Add a New API Endpoint

### Step 1: Define request/response types in `api_models`

```rust
// crates/api_models/src/my_feature.rs
#[derive(Serialize, Deserialize)]
pub struct MyFeatureRequest { … }

#[derive(Serialize, Deserialize)]
pub struct MyFeatureResponse { … }
```

### Step 2: Add domain logic in `router/src/core/my_feature.rs`

```rust
pub async fn create_my_feature(
    state: SessionState,
    req: api_models::my_feature::MyFeatureRequest,
) -> RouterResponse<api_models::my_feature::MyFeatureResponse> { … }
```

### Step 3: Add route handler in `router/src/routes/my_feature.rs`

```rust
pub async fn my_feature_handler(
    state: web::Data<AppState>,
    req: HttpRequest,
    body: web::Json<MyFeatureRequest>,
) -> HttpResponse {
    let flow = Flow::MyFeature;
    Box::pin(api::server_wrap(flow, state, &req, body.into_inner(), |state, auth, body, _| {
        core::my_feature::create_my_feature(state, body)
    }, auth::ApiKeyAuth, api_locking::LockAction::NotApplicable))
    .await
}
```

### Step 4: Register in `routes/app.rs`

```rust
web::resource("/my_feature").route(web::post().to(my_feature::my_feature_handler))
```

---

## 11. Testing

| Type | Location | How to run |
|---|---|---|
| Unit tests | Inline in each crate (`#[cfg(test)]`) | `cargo test` |
| Integration tests | `crates/router/tests/` | `cargo test -p router` |
| Connector tests | `crates/router/tests/connectors/` | Requires sandbox credentials |
| API E2E tests | `cypress-tests/` (v1), `cypress-tests-v2/` (v2) | `npm run cypress` |
| Postman | `postman/` | Import to Postman |
| Load tests | `loadtest/` | see scripts |

Key test utilities in `crates/test_utils/`.

---

## 12. Running Locally

### Docker (Recommended)

```bash
git clone https://github.com/juspay/hyperswitch
cd hyperswitch
scripts/setup.sh
# Choose "Standard" profile → starts app + control center
```

### From Source

```bash
# 1. Install dependencies (Rust, PostgreSQL, Redis, diesel_cli, protobuf)
#    See docs/try_local_system.md

# 2. Set up database
diesel migration run --database-url postgres://db_user:db_pass@localhost/hyperswitch_db

# 3. Copy and edit config
cp config/development.toml config/config.toml
# Edit database and redis credentials

# 4. Run the server
cargo run --bin router -- -f config/config.toml

# 5. Run the drainer (separate terminal)
cargo run --bin drainer -- -f config/config.toml

# 6. Run the scheduler (separate terminal)  
cargo run --bin scheduler -- -f config/config.toml
```

### Useful `just` Commands

```bash
just fmt          # format code
just clippy       # run linter
just migrate      # run DB migrations
just api-docs     # generate OpenAPI spec
```

### Environment Variable Overrides

Any config key can be overridden with env vars using double-underscore as separator:
```bash
ROUTER__SERVER__PORT=9090 cargo run --bin router
ROUTER__REDIS__HOST=myredis.example.com cargo run --bin router
```
