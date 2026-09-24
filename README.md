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
├─ PIE Prospective Shadow
├─ BEJEWELY Security Boundary
└─ BEJEWELY Supply Chain Security

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

`BEJEWELY Security Boundary`와 `BEJEWELY Supply Chain Security`는 특정 Track의 전용 검증기가 아니라 여러 개발축에서 공통으로 실행되는 shared security gate이므로 Project-wide로 유지합니다. PR/branch가 특정 Track에서 이 gate를 촉발하더라도 workflow 책임 자체를 해당 Track으로 바꾸지 않습니다.

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
