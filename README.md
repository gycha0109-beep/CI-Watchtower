# CI Watchtower

Windows 시스템 트레이에 상주하면서 GitHub Actions를 **ChatGPT 작업 트랙 단위**로 감시하는 로컬 데스크톱 앱입니다.

핵심 목적은 긴 CI를 ChatGPT 세션이 붙잡고 기다리지 않게 만드는 것입니다.

```text
ChatGPT 작업
→ PR/Branch에 CI 발생
→ CI Watchtower가 별도 감시
→ ChatGPT 세션 종료/전환
→ GREEN / RED / 장기 실행 / Queue 혼잡 알림
→ 해당 트랙으로 복귀
```

## MVP 기능

- 트랙 등록 / 수정 / 삭제
- 트랙별 GitHub `owner/repo`
- Branch 또는 PR 기준 추적
- 선택적인 workflow 이름 필터
- **장기 CI 기준시간을 트랙마다 사용자가 분 단위 숫자로 직접 입력**
- 현재 head SHA의 workflow run 전체 추적
- `queued / in_progress / completed` 상태 표시
- 대상 workflow가 전부 `success`면 `GREEN`
- `failure / cancelled / timed_out / action_required / startup_failure / stale` 중 하나라도 있으면 `RED`
- 모든 run이 끝났지만 `success` 외 conclusion만 있는 경우 별도 `DONE` 상태
- Windows native notification
- 시스템 트레이 상주 / 창 닫기 시 트레이로 숨김
- 감시 중인 repository 전체의 Running / Queued 수량 (repo-wide)
- Queue 혼잡 임계값 사용자 설정
- 장기 CI 기준 초과 알림(동일 run 중복 알림 방지)
- CI 완료 GREEN/RED 알림(동일 SHA 중복 알림 방지)
- 최근 20회 평균 CI 소요시간
- GitHub PR / Actions 바로가기
- GREEN 완료 트랙 자동 정리 옵션
- 같은 repo/branch 또는 PR을 여러 트랙이 감시할 경우 polling cycle 내부 조회 결과 재사용
- 활성 트랙 / 유휴 상태 adaptive polling
- SQLite 로컬 상태 저장
- GitHub PAT을 OS credential store에 저장. 설정 파일/SQLite 평문 저장 금지

## 기술 기준

2026-09-21 확인 기준:

- Tauri CLI 2.11.5 / Tauri 2.11.x
- React 19.3.0
- Vite 8.3.0
- TypeScript 7.0.2
- Rust backend
- SQLite (`rusqlite`, bundled SQLite)
- Windows Credential Manager 등 OS keyring (`keyring` crate)
- GitHub REST API

## GitHub PAT

Fine-grained Personal Access Token을 권장합니다.

Repository access는 CI를 감시할 저장소만 선택하고 다음 권한만 부여하십시오.

- **Actions: Read** — workflow run 상태 조회
- **Contents: Read** — branch의 현재 head SHA 확인
- **Pull requests: Read** — PR 번호 기반 감시를 사용할 때 필요
- **Metadata: Read** — GitHub가 기본적으로 제공하는 repository metadata read

쓰기 권한은 필요하지 않습니다.

PAT은 앱의 SQLite/JSON/config에 저장하지 않습니다. Rust의 OS keyring adapter를 통해 Windows에서는 Credential Manager 계열 저장소를 사용합니다.

## 동작 방식

### Branch 트랙

```text
repo + branch
→ GET /repos/{owner}/{repo}/commits/{branch}
→ head SHA
→ GET /repos/{owner}/{repo}/actions/runs?head_sha={sha}
→ 현재 SHA의 workflow run 집합
```

### PR 트랙

```text
repo + PR number
→ GET /repos/{owner}/{repo}/pulls/{pr}
→ PR head SHA
→ GET /repos/{owner}/{repo}/actions/runs?head_sha={sha}
→ 현재 SHA의 workflow run 집합
```

workflow 필터를 입력하면 run 이름에 대해 case-insensitive substring 필터를 적용합니다. 여러 필터는 쉼표로 구분할 수 있습니다.

예:

```text
CI,integration
```

### GREEN / RED 판정

```text
대상 run 없음
→ WAITING

하나라도 in_progress
→ RUNNING

하나라도 queued/requested/pending
→ QUEUED

failure-like conclusion 하나 이상
→ RED

모든 run completed + 모든 conclusion=success
→ GREEN

모든 run completed + success 외 non-failure conclusion 포함
→ DONE
```

## Polling / Queue 보호

기본값:

```text
활성 트랙 있음: 25초
유휴 상태: 90초
Queue 혼잡: queued 6개 이상
```

세 값은 앱 UI에서 변경할 수 있습니다.

하나의 polling cycle에서 동일 `repo + branch` 또는 `repo + PR` source는 한 번만 GitHub에서 읽고, 여러 ChatGPT 트랙이 결과를 공유합니다. 또한 각 감시 repository의 최근 Actions run은 repository당 한 번만 읽어 repo-wide Running/Queued 수를 계산합니다. 따라서 다른 SHA에 밀려 있는 queue도 혼잡 경고에 포함하면서, 같은 PR을 여러 트랙에서 본다고 API 요청이 트랙 수만큼 증폭되지는 않습니다.

## 알림

- `[트랙] CI 완료 — GREEN`
- `[트랙] CI 완료 — RED`
- `[트랙] 장기 CI 감지` — 해당 트랙에서 직접 입력한 분 기준 초과
- `GitHub Actions Queue 혼잡`

Tauri의 Windows native notification은 **설치된 앱에서 정상 앱 이름/아이콘으로 표시**됩니다. 개발 실행에서는 PowerShell 이름/아이콘 등으로 보일 수 있습니다.

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

## 로컬 데이터

앱 데이터 디렉터리에 다음만 저장합니다.

```text
ci-watchtower.sqlite3
```

내용:

- track registry
- 현재 track state
- run history
- notification dedupe keys
- polling / queue settings

**GitHub PAT은 이 DB에 저장하지 않습니다.**

## 검증

PR과 `main` push에서 두 workflow를 사용합니다.

- `Fast Check` — Ubuntu에서 frontend TypeScript/Vite build를 빠르게 검증합니다.
- `CI` — Windows에서 frontend build + Rust tests + Tauri installer build를 수행하고 MSI/EXE를 artifact로 업로드합니다.

Windows installer를 받으려면 성공한 `CI` run의 `ci-watchtower-windows` artifact를 다운로드하십시오.

실제 private repo PAT을 설정한 뒤 다음 smoke test를 수행합니다.

1. queued → running → green
2. failed run → red
3. 사용자 지정 장기 CI 기준 초과
4. queue threshold crossing
5. 앱 창을 닫아도 tray polling 지속
6. GREEN 자동 정리
7. PAT 삭제 후 API 호출 fail-closed
