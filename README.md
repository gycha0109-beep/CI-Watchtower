# CI Watchtower

Windows 시스템 트레이에서 GitHub Actions를 **Project → Track → Repository** 단위로 분류하고 감시하는 로컬 데스크톱 앱입니다.

## v0.3 구조

```text
Project
├─ Repositories
│  ├─ repo A
│  └─ repo B
├─ Project-wide CI
│  ├─ CI
│  ├─ Governance
│  └─ Web PR Domain Gates
└─ Tracks
   ├─ ops
   ├─ product-commerce
   ├─ saju
   └─ frontend-integration
```

Project는 제품/서비스 단위, Repository는 GitHub 저장소, Track은 병렬 개발 작업축입니다. 특정 Track에 속하지 않는 정상적인 총괄 Workflow는 `Project-wide CI`로 분리합니다. 따라서 `미귀속`은 실제로 귀속 판단이 필요한 예외 Inbox만 의미합니다.

기존 데이터는 최초 v0.3 실행 시 자동 migration됩니다. MyeongHa/Saju 저장소가 존재하면 `명하` Project로 묶고 기존 Track/Repository를 해당 Project 아래로 승격합니다.

## v0.3.1 BEJEWELY 기본 Registry

v0.3.1부터 로컬 DB migration이 비주얼리 Project를 기본 등록합니다.

```text
Project: 비주얼리 (visualy)
Repository: gycha0109-beep/K_beauty

Project-wide CI
├─ BEJEWELY Current Main Health
└─ PIE Prospective Shadow

Dynamic shared producer
├─ BEJEWELY Security Boundary
├─ BEJEWELY Supply Chain Security
├─ BEJEWELY AI Provider Runtime
└─ BEJEWELY Database Integration Authority
   (+ Admin / Product Offer / Product Data Pipeline / Recommendation Admission)

Tracks
├─ CI Watchtower / CI 운영 정리        → ops
├─ 데이터 정렬 & AI                    → taxonomy-ai
├─ 신규 상품 신뢰도 운영 파이프라인    → trust
├─ Face Lab 연구                       → face-research
├─ Premium Full Report                 → full-report
└─ Mobile                              → mobile
```

기존 producer에서 사용된 `taxonomy&AI`는 Track Key 문자 규칙과 맞지 않으므로 Watchtower가 과거 evidence를 읽을 때 `taxonomy-ai`로 정규화합니다. 신규 producer는 `taxonomy-ai`를 사용합니다.

K_beauty가 기존 Project 아래에 등록되어 있던 경우 Repository를 `비주얼리`로 이동하고, 다른 Project Track에 남아 있는 잘못된 Run 귀속을 정리합니다. 등록된 Project-wide Workflow의 기존 자동 귀속 Run도 공용 CI로 즉시 재분류하며, 수동 귀속은 보존합니다.

`BEJEWELY Security Boundary`와 `BEJEWELY Supply Chain Security`는 특정 Track의 전용 검증기가 아니라 여러 개발축에서 공통으로 실행되는 shared producer이므로 repository-scoped Dynamic으로 유지합니다. Dynamic 선언은 Workflow의 공유 책임만 나타내며 개별 Run의 Track을 강제하지 않습니다.

## Project-wide CI

명하 migration은 다음 Workflow 이름을 Project 전체 공용 CI 규칙으로 등록합니다.

```text
CI
Governance
Web PR Domain Gates
PIE Prospective Shadow
```

또한 `gycha0109-beep/MyeongHa` Repository에는 다음 shared gate를 Repository 범위 Project-wide 규칙으로 등록합니다.

```text
DB Content Reading Suite
DB Runtime Authority Suite
DB PostgreSQL 17 Authority Suite
Supabase Production
Web Browser Smoke
Web Auth Browser Regression
```

이 Repository-scoped 규칙은 같은 이름의 Workflow가 Saju 등 다른 Repository에 존재하더라도 전파되지 않습니다. 자동 Track 귀속은 Project-wide로 정리하지만 수동 귀속은 보존합니다.

공용 CI 규칙은 UI에서 추가/삭제할 수 있으며 프로젝트 전체 또는 특정 Repository 범위로 제한할 수 있습니다. 미귀속 Inbox의 Run에서 **공용 CI로 분류**를 선택하면 같은 Project의 동일 Workflow 이름을 공용 규칙으로 학습합니다.

## Track Key

Track Key는 repository나 ChatGPT 대화 번호가 바뀌어도 같은 작업축이면 유지합니다.

예:

- `frontend-integration`
- `saju`
- `face-reading`
- `face-research`
- `ops`
- `product-commerce`
- `pipeline-reliability`

v0.3 migration은 기존 `commerce` Track Key를 `product-commerce`로 정규화합니다. 과거 evidence 호환을 위해 `commerce → product-commerce`, `privacy-recovery → ops` alias를 유지합니다.

## Producer Contract

개발 작업은 가능한 한 GitHub에 Track Key 흔적을 남깁니다.

### Branch

```text
feat/frontend-integration/reader-scene
fix/ops/privacy-recovery
research/face-research/repeatability
```

### PR body / commit footer

```text
Watchtower-Track: frontend-integration
```

### workflow_dispatch / run-name

```yaml
on:
  workflow_dispatch:
    inputs:
      watchtower_track:
        description: CI Watchtower Track Key
        required: false
        type: string

run-name: "[WT:${{ inputs.watchtower_track }}] ${{ github.workflow }}"
```

`[WT:<track-key>]`는 Workflow 파일명이 아니라 GitHub Actions **run-name**에 노출되는 귀속 신호입니다.


### Workflow Responsibility Review

- Current Run이 PR/commit/branch marker로 정상 귀속돼도 Workflow 자체의 책임 선언이 없으면 Responsibility Review에 표시합니다.
- 고정 Track-owned는 `[WT:<track-key>]`, Project-wide는 `project_workflow_rules`, 여러 Track이 공유하는 producer는 `dynamic_workflow_rules`로 선언합니다.
- Dynamic 규칙은 Track을 강제하지 않습니다. 각 Run은 계속 명시 신호로 fail-closed 귀속됩니다.
- Saju `MESH6J Manual Browser Capture Surface CI`는 repository-scoped Dynamic Workflow입니다.
- MyeongHa `Production Records Current-Subject Smoke`도 repository-scoped Dynamic Workflow입니다. 운영용 Production smoke로 시작했지만 Records/frontend-integration 변경에도 같은 producer가 실행되므로 특정 Track에 고정하지 않습니다. 비표준 `Watchtower-Track: UI` 같은 신호는 alias로 숨기지 않고 계속 fail-closed drift로 남깁니다.
- Dynamic Workflow 규칙은 앱의 **Dynamic Workflow 규칙** 관리 영역에서 프로젝트/저장소 범위로 추가·삭제할 수 있습니다. repository responsibility map에서 동기화한 기본 Dynamic 계약은 `protected` 규칙으로 표시되어 실수로 삭제되지 않습니다.
- Visualy는 K_beauty의 `docs/ci/workflow-responsibility-map.json` producer class를 기준으로 동기화합니다. `current-main-health`와 `pie-prospective`만 Project-wide이며, Admin / Product Offer / Product Data Pipeline / Security Boundary / Recommendation Admission / Supply Chain Security / AI Provider Runtime / Database Integration은 canonical Track을 새로 만들지 않고 repository-scoped Dynamic Workflow로 유지합니다.

### Responsibility Map Drift (v0.3.18)

Repository에 `docs/ci/workflow-responsibility-map.json`이 있으면 polling 시 해당 producer contract를 읽어 WatchTower의 책임 선언과 **detect-only**로 비교합니다.

- repository map에는 있는데 WatchTower에 없으면 `MissingInWatchTower`
- Project-wide / Dynamic 종류가 다르면 `ResponsibilityKindMismatch`
- `static:<track>` binding과 producer의 `[WT:<track>]` 선언이 다르면 `TrackBindingMismatch`
- repository map에서 빠졌지만 WatchTower 규칙과 실제 Run 기록이 남아 있으면 `StaleInWatchTower`
- drift 감지는 Track, Project-wide/Dynamic 규칙, Run assignment를 자동 생성·수정·삭제하지 않습니다.
- map 조회가 일시 실패하면 기존 CI polling/resolution을 실패시키지 않고 마지막으로 성공한 snapshot을 유지합니다.

### Responsibility Map Source Health (v0.3.19)

Drift 0건이 곧 source 정상이라는 잘못된 결론으로 이어지지 않도록 repository별 map source 상태를 별도로 추적합니다.

- `synced`: 현재 poll에서 map을 정상 읽었고 snapshot을 갱신했습니다.
- `not_found`: repository에 map이 없습니다. 이전 snapshot이 있다면 stale-source 경고로 취급하고, snapshot 자체가 없으면 `not configured` 상태로 표시하여 clean drift로 오인하지 않습니다.
- `error`: GitHub/API/parse 조회 실패입니다. 마지막 정상 snapshot과 `last_success_at`은 보존합니다.
- `pending`: v0.3.19 이후 아직 source poll을 수행하지 않은 상태입니다.
- source 실패는 Actions run polling이나 attribution을 중단시키지 않지만, UI는 stale/unavailable source가 있을 때 drift 0건을 clean으로 표시하지 않습니다.

### Responsibility Review Inbox (v0.3.21)

Responsibility Map Drift가 발생하면 단순 상태 행 대신 검토 가능한 Inbox 카드로 표시합니다.

- **Repository contract**: responsibility map binding, source path, workflow path를 함께 표시합니다.
- **WatchTower declaration**: 현재 Project-wide / Dynamic / static Track 관측 상태를 표시합니다.
- **충돌 이유**: 왜 Missing / Stale / Kind / Track mismatch로 판정됐는지 backend가 판정 근거를 내려줍니다.
- **권장 조치 유형**: Project-wide/Dynamic 선언 검토, 재분류 검토, stale 규칙 유지/제거 검토, Track/Map 계약 검토, producer run-name 수정 검토 등을 구분합니다.
- 권장 조치는 **review hint**일 뿐입니다. Inbox는 Track 생성, 규칙 추가/삭제/재분류, producer YAML 수정, Run assignment 변경을 자동 수행하지 않습니다.

### Manual Responsibility Resolution (v0.3.22)

Review Inbox에서 사용자가 명시적으로 승인한 WatchTower 내부 변경만 실행할 수 있습니다.

- `Missing Project-wide` / `Missing Dynamic`: repository-scoped 규칙 추가를 Preview 후 승인합니다.
- `Project-wide ↔ Dynamic` kind mismatch: 충돌 규칙이 repository-scoped일 때만 transaction으로 원자적 재분류합니다. Project 전체 범위 규칙이면 다른 Repository 영향 가능성 때문에 차단합니다.
- `Stale`: repository-scoped 규칙만 제거할 수 있으며, `보류`는 Drift를 숨기지 않고 Deferred 상태로 유지합니다.
- `Track registry mismatch`, producer run-name mismatch, static 책임 충돌, unsupported map binding은 WatchTower 내부 자동 변경 대상이 아니며 Blocked로 표시합니다.
- Preview에는 적용될 변경과 불변 조건을 표시하고, 승인 시 fingerprint를 다시 검증합니다. Preview 이후 계약이 달라졌으면 `stale_rejected`로 fail closed 합니다.
- 실행 후 Responsibility Drift를 다시 계산해서 사라진 경우에만 `resolved`로 기록합니다.
- `responsibility_resolution_audit`에 before/after binding, action, fingerprint, result를 기록합니다.
- Resolution은 canonical Track을 생성하지 않고 producer YAML / repository responsibility map을 수정하지 않으며 manual run assignment를 보존합니다.

### Resolution History / Audit Trail (v0.3.23)

Manual Resolution의 판단과 결과를 immutable event history로 추적합니다.

- 성공한 Resolution도 `after_watchtower_binding`이 비어 있지 않도록 mutation 직후 실제 WatchTower responsibility binding을 snapshot합니다.
- `requested_fingerprint`와 `current_fingerprint`를 분리해 Preview 이후 상태가 바뀐 `stale_rejected` 사건을 명확히 설명합니다.
- `expected_repository_binding`과 `resulting_watchtower_binding`을 보존해 Repository authority → 승인 action → 실제 WatchTower 결과를 복원할 수 있습니다.
- `Deferred → Resolved`는 기존 audit row를 갱신하지 않고 별도 event를 추가합니다.
- Dashboard의 Resolution History는 현재 Project / Repository scope를 따르며 Resolved, Deferred, Blocked, Stale, Open, Failed 필터를 제공합니다.
- Review Inbox의 같은 `review_key`에 과거 이력이 있으면 이전 검토 횟수와 최신 결과를 연결해서 보여줍니다.
- 상세 Audit에서는 before/after contract, action, requested/current fingerprint, actor, audit id와 resolution safety invariant를 확인할 수 있습니다.
- transaction 실행 실패는 rollback 후 `failed` audit event를 남기며 canonical Track, producer YAML, repository responsibility map, manual assignment는 Resolution 경로에서 변경하지 않습니다.

### Responsibility Re-review Queue (v0.3.24)

처리되지 않은 Responsibility resolution을 다시 운영 큐로 회수합니다.

- 같은 fingerprint의 최신 event가 `deferred`면 Inbox 상태를 `DEFERRED`로 유지합니다.
- `failed`, `still_open`, `stale_rejected`처럼 사람이 다시 판단해야 하는 사건은 `ATTENTION`으로 표시합니다.
- 사용자가 `재검토`를 누르면 현재 drift와 fingerprint를 다시 검증한 뒤 `OPEN`으로 되돌리고 즉시 최신 Resolution Preview를 엽니다.
- 재검토 재개 자체도 기존 audit row를 수정하지 않고 `action=reopen`, `result=still_open` event로 추가합니다.
- 재검토 요청 사이에 drift fingerprint가 바뀌면 mutation 없이 `stale_rejected`로 기록합니다.
- Blocked 계약은 재검토 명령으로 우회할 수 없으며 기존 fail-closed 경계를 유지합니다.
- Inbox 상단은 Open / Deferred / Attention / Blocked 건수를 분리해 운영 우선순위를 보여줍니다.
- Resolution History의 Open 필터에서 재검토 재개와 mutation 후 still-open 사건을 함께 추적할 수 있습니다.

### Responsibility Review Aging / Priority (v0.3.25)

Review Inbox를 단순 상태 목록이 아니라 실제 운영 우선순위 큐로 정렬합니다.

- `responsibility_review_state`는 `review_key + fingerprint`별 최초 관측 시각과 최근 관측 시각을 보존합니다.
- Aging은 최초 관측 기준으로 `FRESH(<24h)`, `AGING(24~71h)`, `OVERDUE(>=72h)`로 계산합니다.
- 우선순위는 `P0=ATTENTION`, `P1=72시간 이상 OPEN`, `P2=일반 OPEN`, `P3=DEFERRED`, `BLOCKED` 순으로 분리합니다.
- 같은 우선순위 안에서는 오래된 항목, 실패 시도가 많은 항목을 먼저 노출합니다.
- 각 Inbox 카드에 first seen, review event 수, failed attempt 수, last reviewed 시각을 표시합니다.
- 실패 시도는 `failed`, `stale_rejected`, 실제 mutation 뒤 남은 `still_open`을 집계하며 단순 `reopen` event는 실패로 세지 않습니다.
- fingerprint가 바뀌면 새로운 review 상태로 취급하므로 과거 drift의 aging이 새 계약 상태에 잘못 이어지지 않습니다.
- 기존 fail-closed 경계, repository scope, canonical Track, producer YAML, responsibility map, manual assignment 불변 조건은 유지합니다.

### Responsibility Review SLA / Escalation (v0.3.26)

Aging/Priority 위에 review SLA와 escalation 계층을 추가합니다.

- SLA는 같은 review key + fingerprint의 최초 관측 시각을 기준으로 계산합니다. fingerprint가 바뀌면 새 SLA가 시작됩니다.
- P0=24h, P1=96h, P2=72h review target을 사용하고 P3/DEFERRED, BLOCKED는 SLA exempt로 둡니다.
- P0는 12시간 이하, P1/P2는 24시간 이하가 남으면 DUE_SOON, target을 넘으면 BREACHED입니다.
- P0/P1은 breach 전 WARNING, breach 후 CRITICAL escalation으로 분리합니다. P2는 SLA 상태만 계산하고 escalation queue에는 올리지 않습니다.
- Dashboard의 SLA Escalation 영역은 P0/P1만 별도로 모아 Review Inbox 전체를 훑지 않아도 임박/초과 항목을 볼 수 있게 합니다.
- Inbox 정렬은 CRITICAL → WARNING → 일반을 먼저 적용한 뒤 기존 Priority/Aging/실패 시도 순서를 유지합니다.
- SLA는 자동 mutation, 자동 Track 생성, producer YAML 수정, repository map 수정, run assignment 변경을 수행하지 않습니다. 기존 fail-closed 경계를 그대로 유지합니다.

### Responsibility Escalation Delivery (v0.3.27)

SLA escalation을 WatchTower 내부 표시에서 데스크톱 알림 delivery까지 연결합니다.

- 현재 repository responsibility source가 synced인 drift만 알림 대상으로 사용합니다. stale/error/pending source에서는 새 escalation 알림을 내보내지 않습니다.
- WARNING과 CRITICAL을 별도 event level로 취급하며 review_key + fingerprint + event_type으로 영속 중복 방지합니다.
- 같은 drift가 poll마다 반복되어도 이미 EMITTED된 event는 다시 보내지 않습니다. fingerprint가 바뀌면 새로운 review state이므로 새 알림이 가능합니다.
- notification API 호출 실패는 FAILED로 기록하고 최대 3회까지 재시도합니다. 성공은 EMITTED로 기록합니다.
- Dashboard의 Desktop Escalation Delivery에서 최근 delivery 상태, attempt 수, 오류/사유를 확인할 수 있습니다.
- 이 계층은 관측/알림만 수행하며 Track, workflow rule, producer YAML, repository map, run assignment를 자동 변경하지 않습니다.

### Project-scoped Responsibility Review Policy (v0.3.28)

Responsibility Review SLA와 desktop escalation delivery 정책을 Project 단위로 분리합니다.

- 기본값은 v0.3.27과 동일한 P0 24h, P1 96h, P2 72h 및 due-soon 12h/24h/24h입니다.
- Project별로 P0/P1/P2 target과 due-soon window를 저장할 수 있으며 다른 Project에는 전파되지 않습니다.
- Open drift의 P1 승격 시점은 해당 Project의 P2 target을 사용합니다. P1 target은 P2 target보다 커야 합니다.
- WARNING/CRITICAL desktop delivery를 Project별로 각각 켜고 끌 수 있습니다.
- 정책이 저장되지 않은 Project는 default policy를 read-time fallback으로 사용하므로 기존 설치/DB migration에서 동작이 바뀌지 않습니다.
- 정책 변경은 SLA 계산과 알림 delivery에만 영향을 주며 Track/Rule/producer/map/run assignment를 변경하지 않습니다.

### Escalation Acknowledge / Suppression Lifecycle (v0.3.29)

SLA escalation의 delivery transport와 운영자 처리 상태를 분리합니다.

- 각 escalation은 `review_key + fingerprint` 단위로 `ACTIVE / ACKNOWLEDGED / SUPPRESSED` operator lifecycle을 가집니다.
- `ACKNOWLEDGED`는 운영자가 현재 escalation을 확인했다는 뜻이며 Responsibility Drift를 해결하거나 숨기지 않습니다.
- `SUPPRESSED`는 현재 fingerprint의 SLA Escalation 전용 surface와 신규/재시도 desktop delivery를 억제하지만 원래 Review Inbox의 Drift는 그대로 유지합니다.
- `다시 활성`은 같은 fingerprint를 `ACTIVE`로 되돌립니다. 기존 delivery transport 이력(`EMITTED / FAILED`)은 변경하지 않습니다.
- fingerprint가 바뀌면 새로운 lifecycle이므로 과거 ACK/Suppression을 상속하지 않고 다시 `ACTIVE`에서 시작합니다.
- operator action은 actor, timestamp, before/after state와 함께 immutable audit history로 기록하고 Dashboard에서 최근 이력을 확인할 수 있습니다.
- ACK/Suppression은 Track, Project-wide/Dynamic rule, producer YAML, repository responsibility map, run assignment, resolution 결과를 변경하지 않습니다.

### Timed Suppression / Snooze Expiry (v0.3.30)

영구 SUPPRESSED 외에 현재 fingerprint를 일정 시간만 숨기는 snooze를 지원합니다.

- Dashboard에서 1h / 4h / 24h snooze 또는 기존 영구 숨김을 선택할 수 있습니다.
- timed suppression은 `suppressed_until`을 `review_key + fingerprint` lifecycle에 저장합니다.
- 만료된 suppression은 다음 Dashboard/poll 평가에서 자동으로 `ACTIVE`로 복귀하고 actor `system-expiry`의 immutable audit을 남깁니다.
- 같은 fingerprint를 다시 snooze하면 만료 시각을 갱신하고 그 변경도 audit history에 기록합니다.
- fingerprint가 변경되면 기존 suppression과 마찬가지로 만료 시각도 상속하지 않습니다.
- 자동 ACTIVE 복귀는 SLA Escalation surface를 다시 노출하지만 기존 `EMITTED / FAILED` desktop delivery 이력과 dedupe key를 리셋하지 않습니다. 즉 이미 전달된 동일 fingerprint/event 알림을 snooze 만료만으로 반복 발송하지 않습니다.
- timed suppression 역시 Responsibility Drift 자체나 Track/Rule/producer/map/run assignment/resolution 결과를 변경하지 않습니다.

### Scope Consolidation (v0.3.31)

v0.3.30까지 누적된 Responsibility 운영 기능을 삭제하지 않고 UI에서 역할별 작업면으로 분리합니다.

- **Dashboard**: Running / Queued / Project-wide / Track별 Run과 queue 상태를 중심으로 일상 CI 관측만 표시합니다.
- **Issues**: Producer Contract drift, Responsibility Map drift, SLA/escalation, attribution evidence, 미귀속 CI를 한 곳에서 검토합니다.
- **Advanced**: Project/Track/Repository registry, Project-wide/Dynamic rule, GitHub PAT, polling 및 Responsibility SLA 정책을 관리합니다.
- Dashboard의 미귀속 요약이나 Run 귀속 근거에서 Issues로 직접 이동합니다.
- Track 수정은 Advanced로 이동해 registry 편집과 일상 CI 관측을 분리합니다.
- backend schema, resolver, responsibility lifecycle, audit history, migration 데이터는 삭제하거나 재해석하지 않습니다.
- v0.3.31부터 Responsibility/SLA 계층은 신규 제품 축으로 확장하지 않고 고급 진단/운영 기능으로 유지합니다.

## Producer Contract Health

Dashboard는 각 Repository의 **최근 최대 50개 Run**을 evidence 표본으로 유지합니다. 다만 현재 producer 건강도는 표본 전체를 그대로 평균내지 않고, 같은 Repository의 같은 GitHub `workflow_id`에서 **가장 최신 non-ignored Run 하나**만 current producer 상태로 사용합니다.

정상 계약으로 계산하는 경우:

- Project-wide CI 규칙으로 분류된 Run
- `run_name`, `pr_marker`, `commit_marker`, `branch` 명시 신호로 Track에 귀속된 Run

drift bucket으로 분류하는 경우:

- learned fingerprint 기반 `inference`
- 과거 alias migration의 `track_alias`
- 수동 귀속
- 미귀속 / 충돌
- 그 밖의 비표준 귀속

따라서 Project-wide CI는 `[WT:*]`가 없어도 정상이며, shared workflow를 억지로 Track에 넣어 coverage를 올리지 않습니다. 이 지표의 목적은 resolver 성공률이 아니라 **producer가 스스로 귀속 근거를 얼마나 명시적으로 남기고 있는지** 확인하는 것입니다.

Producer Contract 패널은 drift를 두 층으로 분리합니다.

- **Current Drift**: `repository_id + workflow_id` 기준 최신 Run이 비정상 계약인 producer. 상단 coverage와 위험 색상은 이 집합만 기준으로 계산합니다.
- **Historical Drift**: 같은 workflow에 더 최신 Run이 이미 존재하는 과거 비정상 Run. 최근 50-run evidence와 Attribution Audit에서는 계속 확인할 수 있지만 현재 producer 이상으로 계산하지 않습니다.

GitHub가 Dependabot 업데이트를 위해 생성하는 `dynamic/dependabot/*` workflow는 repository의 `.github/workflows` producer가 아니므로 Track 귀속/Producer Contract/미귀속 Inbox 표본에서 제외합니다. Runner 혼잡을 보기 위한 repository Running/Queued 집계에는 그대로 남깁니다.

최근 50-run bucket 통계는 historical evidence를 포함한 표본 통계로 남습니다. 따라서 과거 untagged Run이 표본에 남아 있어도 최신 producer가 정상 계약으로 회복됐다면 현재 coverage를 계속 낮추지 않습니다. 각 drift 행을 선택하면 기존 Attribution Audit에서 resolver evidence와 Historical Reconciliation 이력을 검토할 수 있습니다.

## Resolver

우선순위는 다음과 같습니다.

1. Project-wide CI 규칙
2. Run 이름의 `[WT:<track-key>]`
3. PR 본문의 `Watchtower-Track:`
4. commit footer의 `Watchtower-Track:`
5. branch Track Key
6. 수동 귀속으로 학습된 workflow fingerprint

동일 최고 우선순위의 명시 신호가 서로 다른 Track을 가리키면 `conflict`로 둡니다. 상위 명시 신호가 존재하면 낮은 우선순위 branch 신호 때문에 false conflict를 만들지 않습니다.

기존 미귀속 Run이 GitHub 최근 100개 window에서 밀려나도 로컬 DB의 미해결 Run을 bounded batch로 다시 평가합니다. 새 alias나 PR/commit marker, 공용 CI 규칙이 생기면 과거 Run도 점진적으로 재분류됩니다.

## 미귀속 CI

미귀속 Inbox에서는 다음 처리가 가능합니다.

- 같은 Project의 기존 Track으로 수동 귀속
- Project-wide CI로 분류하고 동일 Workflow 규칙 학습
- 무시
- 원본 GitHub Actions 열기

Dashboard는 미귀속 Run을 최근 200개까지 내려주며, 화면에 **현재 필터 건수 / 전체 DB 건수**를 함께 표시합니다.

## Run ID / rerun

발견된 Run은 branch 최신 HEAD와 분리해 Run ID로 저장합니다.

```text
Run ID 35627433295
Attempt 1 → failure
Attempt 2 → success
```

`run_attempt`을 별도로 기록하고 알림 dedupe도 `track + run_id + run_attempt + event` 기준으로 관리합니다.

## Queue / Polling

기본값:

```text
활성 상태: 25초
유휴 상태: 90초
Queue 혼잡: queued 6개 이상
```

Running은 `in_progress`, Queued는 `queued / requested / pending / waiting`을 집계합니다. Repository 조회 실패 시 stale Running/Queued 수치를 유지하지 않고 0으로 초기화하고 오류를 기록합니다.

## GitHub PAT

Fine-grained PAT 권장 권한:

- Actions: Read
- Contents: Read
- Pull requests: Read
- Metadata: Read

PAT은 SQLite에 저장하지 않고 Windows Credential Manager backend를 사용합니다. 저장 직후 재조회하여 값이 동일한지 검증합니다.

## 로컬 DB v0.3

주요 테이블:

- `projects`
- `project_workflow_rules`
- `watch_tracks`
- `track_aliases`
- `monitored_repositories`
- `workflow_runs`
- `run_attempts`
- `run_assignments`
- `run_evidence`
- `track_fingerprints`
- `notifications_v2`

## 실행

Windows prerequisites:

- Node.js 22+
- Rust stable / Cargo
- Visual Studio C++ Build Tools
- WebView2 Runtime

```powershell
npm install
npm run tauri dev
```

Release installer:

```powershell
npm run tauri build
```

## v0.3 Acceptance

- Project 생성/삭제 및 Repository/Track Project 귀속
- Project → Track → Repository 필터
- Project-wide CI 규칙과 별도 화면
- `CI / Governance / Web PR Domain Gates / PIE Prospective Shadow` 기본 공용 분류
- `commerce → product-commerce` migration
- `privacy-recovery → ops` 과거 key alias
- 명시 marker가 branch보다 우선하여 false conflict 방지
- 최근 100개에서 밀린 과거 미귀속 Run 점진 재평가
- 미귀속 전체 건수와 화면 표시 건수 구분
- 다른 Project Track으로 수동 오귀속 방지
- `repository_id + workflow_id` 최신 Run 기준 Current Drift / Historical Drift 분리
- 과거 drift가 남아 있어도 최신 정상 producer coverage를 낮추지 않음
- Repository-scoped Project-wide 규칙이 다른 Repository에 전파되지 않음
