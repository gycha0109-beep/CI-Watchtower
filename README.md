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

## Project-wide CI

명하 migration은 다음 Workflow 이름을 기본 공용 CI 규칙으로 등록합니다.

```text
CI
Governance
Web PR Domain Gates
PIE Prospective Shadow
```

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
