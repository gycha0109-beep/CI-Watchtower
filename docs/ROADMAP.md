# CI WatchTower — 운영·연동 개발 로드맵

> 기준일: 2026-10-13 (KST)
> 기준 저장소: `gycha0109-beep/CI-Watchtower`
> 현재 구현 기준: **v0.3.39**, `main` `8156508584a79f6a6415398e9d20e59a2f618998`
> 상태: **읽기 전용 Autopilot 브리지 구현·병합 및 CI 통과 / 사용자 Windows 실환경 연결 검증 대기**
> 문서 목적: 채팅방/개발 세션이 변경되어도 실제 저장소와 증거를 확인해 동일한 우선순위로 개발을 이어가기 위한 장기 실행 계획

이 문서는 고정된 트랙 목록을 주입하는 설정 파일이 아니다. **향후 작업의 우선순위와 완료 판정**을 정한다. 실제 구현/CI 상태는 매 작업 시작 시 `main`과 GitHub Actions로 다시 확인하며, 아래 SHA·버전·테스트 수치는 **2026-10-13 시점의 검증 스냅샷**이다.

## 1. 제품 정의와 독립성

CI WatchTower는 다중 저장소·다중 개발 작업의 **GitHub Actions 관측·귀속·진단 프로그램**이다. CI를 직접 실행하는 Runner도, 작업 실행/AI 명령 전달/PR 병합 프로그램도 아니다.

```text
Project
  ├─ Repository (다대다 작업 연관을 고려한 실제 GitHub 저장소)
  ├─ Work Track (생성·변경·분리·종료 가능한 작업축)
  ├─ CI Responsibility (공용 / 동적 / 고정 책임)
  └─ Workflow Run → Attempt → Job → Step (실제 검증 관측)
```

- **Work Track ≠ CI Responsibility**: 같은 PR의 작업 트랙과 해당 PR을 검증한 기술 도메인은 서로 다를 수 있다.
- **Project-wide CI**는 여러 작업 트랙의 근거가 될 수 있지만 임의의 단일 트랙 소유로 재분류하지 않는다.
- 작업 트랙은 변동한다. 이름 변경 시 ID·이력·alias 보존을 우선하고, 분리·통합이나 공동 작업은 근거가 있을 때만 연관시킨다.
- 명시 신호 우선, 미등록/충돌/불충분 근거는 fail-closed. 기술적 Workflow 이름이나 `[WT:*]` 잔재를 토대로 유령 Work Track을 만들지 않는다.
- 사용자별 Project/Repository/Track을 일반 런타임에 영구 seed하지 않는다. 신규 설치는 빈 Registry로 시작한다.
- WatchTower의 SQLite·PAT·알림·기존 분류·수동 귀속·Producer Contract는 Autopilot이 수정하지 않는다.

### 연동 책임 경계

| 구성 요소 | 담당 | 담당하지 않는 것 |
|---|---|---|
| **WatchTower** | Project/Repository/Track, Run/Attempt/Job 관측, 귀속·책임·최신성, 읽기 전용 상태 제공 | 코드 수정, 프롬프트 전송, CI 자동 통과 판단으로 PR 병합 허가 |
| **Development Autopilot** | 작업 세션·실행 대기/재개 정책, WatchTower 연동, 외부 실행 어댑터 | WatchTower 분류 재작성, SQLite 직접 갱신 |
| **Autopilot의 GitHub 검증기** | 현재 PR Head SHA, 필수 Checks, 승인·병합 조건의 독립 확인 | 과거 GREEN이나 단일 Run 성공만으로 전체 완료 판정 |
| **Producer 저장소** | 실제 Workflow YAML, 책임 선언, PR/commit 작업 Track 신호 | WatchTower Registry 자동 강제 변경 |

연관 저장소:
- WatchTower: https://github.com/gycha0109-beep/CI-Watchtower
- Autopilot: https://github.com/gycha0109-beep/Development-Autopilot
- Autopilot 상위 설계: https://github.com/gycha0109-beep/Development-Autopilot/blob/main/docs/ROADMAP.md 및 `docs/ARCHITECTURE.md`
- WatchTower 브리지 계약: [development-autopilot.md](integrations/development-autopilot.md)
- 기존 데스크톱 실사용 검증: [acceptance-smoke.md](acceptance-smoke.md)
- Producer 최신화 경계: [producer-discovery/README.md](producer-discovery/README.md)

## 2. 현재 구현 및 확인된 상태

**v0.3.39 / PR #57**: https://github.com/gycha0109-beep/CI-Watchtower/pull/57

- 별도 Tauri 프로세스 없이 기존 앱에서 `127.0.0.1:48765`에 인증된 로컬 읽기 전용 HTTP 브리지 제공.
- Windows Credential Manager에 GitHub PAT와 다른 무작위 Bearer credential 저장.
- `GET /v1/health`, `/v1/registry`, `/v1/runs`, `/v1/runs/{run_id}` 제공.
- Project/Repository/Track, Workflow/Run/Attempt/Head SHA, Work Track 연관·CI Responsibility, Poll 성공/오류, 데이터 최신성 등 최소 DTO 제공.
- `UNKNOWN / STALE / POLL_ERROR / CONFLICT` 구분. `safeToAdvance=false`를 유지해 WatchTower 단독 응답이 Autopilot 자동 진행을 승인하지 않도록 구성.
- 기존 Dashboard, 폴링, 트랙 분류, DB 기존 이력, Windows 설치 경로 유지.
- PR Fast Check 및 Full CI **SUCCESS**, Rust **70 passed / 0 failed**, Windows 패키징 성공.
- 병합 후 `main` Fast Check **#159 SUCCESS**, Full CI **#159 SUCCESS**. 당시 Open PR 0.
- **미검증 경계**: 사용자의 실제 Windows 계정·기존 설치 DB·PAT·Credential Manager·실제 localhost 인증·실제 Autopilot 프로세스 연결 및 종료/재시작 복구.

**완료를 주장하지 말 것:** 위 CI 성공은 실제 사용자의 Windows PC에서 연동·복원까지 확인했다는 뜻이 아니다.

## 3. 우선순위/의존 관계

```text
P0  기존 v0.3.39 근거 확인
  ↓
P1  Windows 실환경 브리지 Acceptance / 회귀
  ↓
P2  실제 결함 기반 계약 안정화 (필요한 최소 수정만)
  ↓
P3  Development Autopilot 소비자 연결 및 End-to-End 검증
  ↓
P4  변화하는 Work Track의 안전한 동기화·다중 연관 (증거 기반)
  ↓
P5  Workflow → Job → Step 병목 관측 UX (실측 기반)
  ↓
P6  릴리스·복구·운영 마감
```

P1~P3가 **지금의 주 작업축**이다. P4~P6는 장기 계획이며 앞 단계의 실패를 미뤄두고 기능만 늘리는 식으로 수행하지 않는다. 각 Phase는 독립된 검증 가능한 PR로 나눌 수 있다.

## 4. 실행 단계

### P0 — 현재 상태 재검증 (시작점; 저장소 기준 완료, 매 세션 재확인)

1. `main` SHA, 버전, Open PR, 최근 PR/Push CI를 조회한다.
2. 브리지/Registry/Responsibility 관련 문서를 읽고, README의 버전별 설명과 실제 코드가 어긋나는지 체크한다.
3. 미병합 작업, 실패한 CI, 개발 중 타 브랜치와 충돌 가능성이 있으면 먼저 정리한다.

**Exit:** 현재 구현 상태·미해결 실패·다음 적용 지점이 명확하다. 스냅샷을 현재 사실로 무조건 재사용하지 않는다.

### P1 — 실제 Windows 브리지 연결 Acceptance (**최우선 / 아직 사용자 환경에서 검증되지 않음**)

**목표:** 실제 설치된 WatchTower를 Autopilot 또는 독립 테스트 클라이언트가 읽을 수 있음을 입증한다.

1. 기존 설치 위에 v0.3.39 이상을 적용한다. **실사용 SQLite·Windows Credential Manager·PAT는 백업/보존**하고 초기화하지 않는다.
2. 같은 Windows 사용자 컨텍스트에서 WatchTower 실행·트레이 유지 확인; `/v1/health` 인증 성공 / 무인증 401 / 부적절한 `Host` 또는 `Origin` 403 확인.
3. GitHub PAT 등록/조회 경로와 **브리지 전용 자격 증명**이 분리돼 있으며 API 응답·로그·문서에 비밀정보가 없는지 확인.
4. 등록된 Project/Repository/Track을 `/v1/registry`로 읽고, 실제 Repository Run을 SHA·Attempt 지정해 `/v1/runs/{id}`로 읽는다.
5. Running, Queued, Success, Failure, Cancelled/Timed out, Project-wide, Dynamic, Track-owned, Unassigned, Conflict를 존재하는 근거로 검증한다. 실환경에서 즉시 재현 불가한 상태는 deterministic fixture 테스트로 보완하고 구분 보고한다.
6. SHA mismatch, 과거 Attempt, 다른 Track, Run 미존재, stale/poll error가 **안전한 성공으로 바뀌지 않음**을 확인한다.
7. Windows 종료·재시작·트레이 최소화·오프라인·GitHub 오류·포트 점유를 시뮬레이션한다. 연결 실패는 `WATCHTOWER_UNAVAILABLE`로 취급한다.
8. 사전/사후 Registry·수동 귀속·Alias·Run 이력·알림 중복 방지 레코드의 무결성을 비교한다.

**Exit:** 설치된 Windows에서 정상 연결 + 인증 거부 + 재시작/실패 복구의 증거가 있고 **기존 데이터가 보존**됨. 사용자 OS 실행 권한이 없어 수행하지 못한 항목은 **EXTERNAL VALIDATION REQUIRED**로 남긴다. 추측으로 PASS 처리 금지.

**P1 검증 도구 (실행 준비):** [Windows bridge acceptance](integrations/windows-bridge-acceptance.md) 및 `tools/Test-WatchTowerBridge.ps1`. 인증/HTTP 오류·실 Run SHA/Attempt·트레이/종료/재시작·Registry identity baseline을 사용자 Windows에서 검사한다. Rust HTTP 계약 회귀·실제 loopback TCP/Windows CI의 격리된 Credential Manager canary·PowerShell 구문 검증은 CI에서 수행하지만 사용자의 Credential Manager·PAT·기존 DB 복구를 대체하지 않는다. **실사용 증거가 없으면 P1은 `CODE COMPLETE · LIVE PENDING`으로 유지한다.**

### P2 — 브리지 계약의 실제 결함 보완 (P1 발견사항 우선)

- `docs/integrations/development-autopilot.md`와 실제 응답/오류 코드를 1:1 비교하고 샘플 코드로 계약 테스트 수행.
- Head SHA/최신 Attempt, `lastSeenAt`·`lastSuccessfulPollAt`·`lastPollAttemptAt`의 의미와 경과 기준 확인.
- 빈 Run 목록, 부분 표본, 과거 GREEN, 비활성 Track, 서로 다른 Repository 동일 key, Project-wide 공유, 충돌/미귀속에 대해 **fail-closed** 회귀 테스트.
- 포트 충돌·Windows 계정 차이·OS Credential 권한 문제 발생 시 원인 식별·클라이언트 설정 안내를 보강한다. 필요하지 않으면 새로운 설정 UI/인증 체계를 만들지 않는다.
- API breaking change만 major version 상승; non-breaking 필드 추가는 1.x로 제한.
- 문서 오탈자 및 코드-문서 불일치 정리. 기능이 정상인데 테스트만 부족하면 테스트/문서에 집중한다.

**Exit:** 확인된 결함 재현 테스트 PASS, 기존 Rust/프런트/Windows CI GREEN, 기존 Tauri 명령/DB/폴링 불변.

### P3 — Development Autopilot 소비자 연결 (별도 저장소 의존)

작업 주체는 원칙적으로 `Development-Autopilot`이다. WatchTower에서는 **추가 공개 인터페이스가 실제로 부족할 때만** 최소 범위로 대응한다.

1. Autopilot `docs/ROADMAP.md` Phase 3 및 `docs/ARCHITECTURE.md` 최신 내용 확인.
2. 같은 Windows 사용자 자격 증명으로 브리지 토큰을 취득하고, 버전 검사·타임아웃·연결 실패를 다루는 **읽기 전용 adapter** 구현.
3. Autopilot Workstream을 WatchTower Project/Repository/Work Track과 명시적으로 매핑하되, 동적 Track 변경 때 무조건 오래된 mapping을 승계하지 않는다.
4. 특정 PR 번호·현재 Head SHA·Run ID·최신 Attempt의 **정확한 일치 여부를 GitHub 검증기와 교차 확인**한다.
5. 단일 성공 Run이나 `completeCoverage=false` Run 목록을 전체 required checks의 성공으로 간주하지 않는다. Project-wide CI는 여러 작업에 연관될 수 있으나 임의 소유권 변경 금지.
6. `CONTINUE`, `WAIT_CI`, `ANALYZE_FAILURE`, `CALL_OPERATOR` 등 소비자 판단은 Autopilot 책임. WatchTower는 상태 및 근거만 제공한다.
7. 실제 1개 PR / 1개 Workstream의 `queued → running → completed` 관측, 이전 GREEN 재사용 방지, 다른 Track 성공의 오적용 방지를 검증한다.

**Exit:** Autopilot의 mock/계약 테스트 PASS, 실제 연결 smoke PASS, 부정확하거나 오래된 CI 상태에서 자동 진행 **0건**. AI 실행·PR 병합 권한을 WatchTower에 추가하지 않는다.

### P4 — 유동적인 Track 및 교차 작업 연관 (필요성 검증 후)

- 생성/이름 변경/비활성/분리/통합/다중 Track 참여를 **실제 PR evidence와 기존 Registry**에 맞춰 검토한다. 자동 강제 동기화나 개인 프로젝트 하드코딩은 금지.
- Work Track과 CI Responsibility의 2축은 유지한다. 공동 작업을 표현하려면 1개 PR의 **주 트랙과 복수 연관 트랙**을 나누되, 기존 단일 귀속 필드와 migration 호환성을 먼저 검증한다.
- ID·이력·별칭·수동 귀속·불변 audit을 보존한다. 충돌하거나 새로운 Track의 근거가 부족하면 후보/미귀속으로 유지한다.
- MyeongHa, Saju, K_beauty는 회귀 실사례로 사용하되 앱 런타임에 이 저장소들의 이름/키를 박지 않는다.

**Exit:** 변경/분리/통합 테스트와 실제 Producer 사례에서 잘못된 트랙 재귀속 0, 이력 손실 0. 근거·수요가 없으면 **DEFERRED** 가능.

### P5 — CI 병목을 직접 설명하는 관측 화면 (필요성 검증 후)

- 기존 `get_run_jobs`의 Attempt별 Job/Step 관측을 최대한 재사용한다.
- Workflow 총 소요, Job 대기, 실제 Step 실행, Job 의존 대기를 **측정 가능한 구간만** 분리한다.
- `needs` DAG 데이터가 없으면 Runner 대기와 Dependency 대기를 **추정으로 단정하지 않는다**.
- 재실행·취소 폭증, 같은 Head에서 여러 Workflow fan-out, 최초 Job 대기 등을 제한된 최근 Run 표본에서 보여준다.
- 과도한 GitHub API/Rate Limit 요청 방지: 현재 실행 중 + 최근 상세 조회만 우선하고 백그라운드 전수 수집은 지양한다.

**Exit:** 사용자가 “CI가 왜 14분 걸렸는지”를 실제 Job/Step 근거로 구분 가능. 알 수 없는 원인은 Unknown으로 표기.

### P6 — 장시간 운용·복구·릴리스 마감

- 기존 DB/자격 증명/설정/알림 dedupe 보존, 백업 및 기존 버전 위 재설치 검증.
- WatchTower 종료, 트레이 전환, 절전 복귀, 네트워크 단절, rate limit, 포트 충돌에서 상태 신뢰성 점검.
- Test/CI/Windows NSIS·MSI·EXE 산출물과 실제 사용자 PC 설치 smoke를 구분해서 기록.
- 운영 판단에 필요하지 않은 Governance/SLA 신규 기능은 추가하지 않는다.

**Exit:** 장시간 관측 시 상태 오판·중복 알림·데이터 손실에 대한 재현/검증 기록 확보.

## 5. 공통 구현·검증 규칙

모든 Phase에서 아래 순서를 따른다.

1. 최신 `main`, 열린 PR, 기존 테스트/CI, 관련 문서 재확인.
2. **문제·근거·변경 범위·영향·완료 기준을 먼저 정의**하고 기존 기능이 이미 해결하는지 확인.
3. 최소 범위 feature/fix 브랜치에 구현. 실사용 데이터/SQLite 스키마를 변경할 때는 백업·호환성·one-shot migration 및 롤백 경로 확인.
4. 테스트: 최소 Rust 회귀, 프런트 build, 인증/경계/귀속 fixture, 필요 시 Windows 실제 smoke.
5. PR 설명에 목적/변경/검증/남은 리스크 기록, `Watchtower-Track: ops` 사용.
6. PR Fast Check + Full CI 완료 확인, exact HEAD SHA 및 mergeable 확인. 기존 저장소 정책·필요 승인 준수 후 squash merge.
7. 병합 후 **main** Fast Check + Full CI 확인. Windows installer artifact 생성과 파일 존재를 확인한 뒤 전달.
8. 로드맵 단계별로 **완료 / 중요 미해결 / 외부 검증 대기 / 다음 실행 항목**을 문서에 갱신. PR 생성만으로 완료 처리하지 않는다.

테스트는 모의 결과와 실제 GitHub/Windows 관측을 분리해서 보고한다. 자동 테스트 PASS가 OS Credential·localhost 네트워크·실제 UI 동작을 자동으로 입증하지 않는다.

## 6. 작업 종료 조건과 다음 실행 항목

- **DONE**: 요구된 코드/문서/CI/실환경 검증까지 갖춘 경우.
- **CODE COMPLETE · LIVE PENDING**: PR/CI는 통과했지만 사용자의 Windows 환경 검증이 남은 경우.
- **BLOCKED / EXTERNAL**: OS 권한, 사용자 Credential, 설치된 DB, 실제 GitHub 계정 등 외부 환경이 필요한 경우. 실행 불가 사항은 정확히 보고한다.
- **FAILED**: 필수 회귀 또는 안전성 조건 미충족. 성공으로 포장하지 않는다.
- **DEFERRED**: 이후 우선순위로 둔 기능. 앞 단계 통과와 무관한 기능 확장 금지.

### 새 개발 세션의 첫 작업

1. 이 문서 및 [브리지 계약](integrations/development-autopilot.md), `main`과 최근 CI를 다시 읽는다.
2. **P1 Windows 실환경 연결 smoke**를 진행할 수 있는지 확인한다. 테스트 환경이 없으면 fixture 기반 계약 테스트/간결한 smoke 도구·문서와 실제 확인 지점을 준비하되, 실제 PASS로 표기하지 않는다.
3. 발견한 실제 결함이 있을 때만 P2의 좁은 수정 PR을 연다.
4. Autopilot 소비자 측 구현은 해당 저장소/트랙의 권한과 작업 상태를 확인해 P3으로 진행한다.
