# CI Watchtower

Windows 시스템 트레이에서 GitHub Actions를 **개발 Track Key 단위**로 자동 분류하고 감시하는 로컬 데스크톱 앱입니다.

v0.2부터 PR 번호나 branch HEAD를 감시 대상으로 직접 등록하지 않습니다. 사용자는 감시할 repository와 Track Key만 등록하고, Watchtower는 repository 전체의 최근 Actions Run을 수집한 뒤 명시적 Track Key 신호를 기준으로 각 트랙에 귀속합니다.

## v0.2 핵심 구조

```text
Monitored repositories
  ├─ gycha0109-beep/Saju
  └─ gycha0109-beep/MyeongHa
              ↓
       Repository Run Collector
              ↓
          Run ID pinning
              ↓
          Track Resolver
       ┌──────┼─────────┐
       saju   ops   frontend-integration
```

Repository는 감시 대상, Track은 분류 대상, GitHub Actions Run ID는 실제 추적 대상입니다. main HEAD가 이동하거나 트랙이 다른 repository로 이동해도 이미 발견한 Run은 Run ID로 계속 추적합니다.

## Track 등록

트랙에는 다음 세 값만 필요합니다.

```text
트랙 이름: 프론트 연동
Track Key: frontend-integration
장기 CI: 8분
```

Track Key는 repository나 ChatGPT 대화 번호가 바뀌어도 같은 작업축이면 유지합니다.

예:

- `frontend-integration`
- `saju`
- `face-reading`
- `face-research`
- `ops`
- `commerce`
- `pipeline-reliability`

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

### workflow_dispatch

dispatch workflow는 선택 입력값을 받을 수 있게 구성합니다.

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

Run 이름에 `[WT:<track-key>]`가 노출되면 Watchtower가 PR 연결 여부와 관계없이 직접 귀속할 수 있습니다.

## Resolver 우선순위

명시 신호를 우선합니다.

1. Run 이름의 `[WT:<track-key>]`
2. PR 본문의 `Watchtower-Track:`
3. commit footer의 `Watchtower-Track:`
4. branch segment의 Track Key
5. 사용자가 이전에 수동 귀속해서 학습된 workflow fingerprint

명시 신호가 서로 다른 Track Key를 가리키면 자동 귀속하지 않고 `conflict`로 둡니다. 확정 근거가 없으면 `미귀속 CI` Inbox에 남깁니다.

Workflow 이름 하나만으로는 자동 확정하지 않습니다.

## 미귀속 CI

미귀속 Inbox에서 Run을 기존 트랙에 수동 연결하거나 무시할 수 있습니다.

수동 연결 시 해당 repository의 workflow 이름을 weight 50 보조 fingerprint로 학습합니다. 이 fingerprint 단독으로는 자동 귀속 임계값 70을 넘지 못하므로 잘못된 자동 귀속을 방지합니다.

## Run ID / rerun

발견된 Run은 branch의 최신 HEAD와 분리하여 Run ID로 저장합니다.

```text
Run ID 35627433295
Attempt 1 → failure
Attempt 2 → success
```

`run_attempt`을 별도로 기록하고 알림 dedupe 키도 `track + run_id + run_attempt + event` 기준으로 관리합니다.

## Queue / 상태

Repository 전체 최근 Run에서 다음 상태를 집계합니다.

- Running: `in_progress`
- Queued: `queued / requested / pending / waiting`
- GREEN: 완료 + success
- RED: failure / cancelled / timed_out / action_required / startup_failure / stale
- DONE: 그 외 terminal conclusion

Repository 조회 실패 시 이전 Running/Queued 값을 그대로 보여주지 않고 0으로 초기화하며 오류를 저장합니다.

## Polling

기본값:

```text
활성 상태: 25초
유휴 상태: 90초
Queue 혼잡: queued 6개 이상
```

한 repository에서 최근 100개 Run을 한 번 가져온 뒤 로컬 DB에 upsert합니다. commit/PR evidence 조회는 polling cycle 안에서 SHA 기준으로 캐시합니다.

## GitHub PAT

Fine-grained PAT 권장 권한:

- Actions: Read
- Contents: Read
- Pull requests: Read
- Metadata: Read

PAT은 SQLite나 설정 파일에 저장하지 않습니다. Windows에서는 OS Credential Manager backend를 사용합니다. 저장 후 즉시 다시 읽어 값이 동일한지 검증합니다.

## 로컬 DB v0.2

주요 테이블:

- `watch_tracks`
- `monitored_repositories`
- `workflow_runs`
- `run_attempts`
- `run_assignments`
- `run_evidence`
- `track_fingerprints`
- `notifications_v2`

기존 v0.1 `tracks` 데이터는 최초 실행 시 v0.2 구조로 보존 migration합니다. 알려진 트랙 이름은 안정적인 Track Key로 변환하고 기존 repository는 전역 감시 저장소로 승격합니다.

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

## v0.2 Acceptance

- branch Track Key 자동 귀속
- PR `Watchtower-Track` 자동 귀속
- `[WT:key]` workflow_dispatch 자동 귀속
- Saju ↔ MyeongHa 이동 후 동일 Track 유지
- main HEAD 변경 후 기존 Run 추적 유지
- rerun attempt 별 상태/알림 분리
- 명시 신호 충돌 시 conflict
- 근거 부족 시 미귀속 Inbox
- 수동 귀속 후 fingerprint 학습
- `waiting` 포함 Queue 집계
- Windows tray 감시 및 native notification
