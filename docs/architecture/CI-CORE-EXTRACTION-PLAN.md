# CI Core 분리 계획 — WatchTower / Development Autopilot / BuildMap

> 작성: 2026-11-01 · 상태: 설계 기준 및 Phase 1 코드 분리 진행 · 구현/배포 승인과 별개
> 기준: WatchTower main a3dc012, PR #66 → #67 → #68 (마지막 확인 HEAD 51552150); Autopilot main 27a7684, CI 관련 열린 PR #81/#82/#83. **최신 GitHub 사실은 각 단계에서 재확인.**

## 1. 코드로 확인한 현재 구조

- `src-tauri/src/lib.rs`: Tauri 앱 시작 시 `init_db`, `autopilot_bridge::start`, `start_poller`를 호출. GitHub PAT 접근, API 클라이언트, Actions 수집, 실행 저장, Track 귀속, 수동 변경 명령 및 SQLite 스키마가 동일 모듈에 존재.
- `src-tauri/src/producer_sync.rs`: Workflow/PR/Track producer 관측과 재조정. 저장소별 메타데이터에서 동적으로 획득하며, 원본 귀속·이력 보존이 핵심.
- `src-tauri/src/autopilot_bridge.rs` (현재 #68 기준): 별도 프로세스가 아닌 Tauri 내 인증된 127.0.0.1:48765 조회 API. read-only SQLite 연결을 이용하며 각 Run의 SHA/Attempt/PR identity, 최신성·충돌·취소 후속 근거 등을 검사. Bridge token은 GitHub PAT와 구분된 Windows Credential Manager 비밀정보.
- `src-tauri/src/job_telemetry.rs`: 현재 Attempt의 Job/Step을 GitHub에서 조회하며 Tauri 명령과 브리지에서 공유.
- `src/api.ts`: Dashboard 및 설정 변경은 `invoke` 호출. 로컬 HTTP 브리지는 조회 전용으로 독립 앱에 제공.
- `tools/verify_watchtower_db.py`, Windows PowerShell smoke: 기존 DB의 Registry/Alias/Run/Attempt/수동 귀속/알림 이력 보존 및 실제 자격증명/실행 검증을 위한 도구.
- Autopilot 소비자 `local/watchtower-client.js`, `watchtower-observer.js`, `github-verifier.js`: 같은 사용자 Windows Credential Manager의 브리지 토큰을 읽고 Run과 GitHub PR 현재 HEAD·필수 검사 등을 **별개로** 검증. PR #83에서 추가적인 취소/대체 관측을 제안 중이며 미병합.
- BuildMap: Next.js 웹앱의 `apps/web/lib/github/api.ts`, GitHub 연동 API 및 결정 이력 화면이 존재. WatchTower 로컬 루프백이나 사용자 Windows 자격증명을 클라우드 BuildMap 서버에서 직접 읽을 수 없음.

**현재 한계:** 앱을 완전히 종료하면 폴링과 포트 리스너도 함께 종료. `main`은 아직 v0.3.40이고 CI 확장 변경은 #66/#67/#68의 **미병합 스택**에 있음. 새로운 구조를 배포했다고 주장할 수 없음.

## 2. 목표 아키텍처와 권장안

최종 권장: **Rust 공통 Core 라이브러리 + 선택 가능한 단일 소유자 실행기(내장/독립) + 읽기 전용 어댑터**.

1. 순수 로직 계층: PR/Run/Attempt identity, 신선도, CI 결론, 책임·귀속을 Tauri 비종속 Rust 라이브러리로 이전.
2. 저장/수집 계층: 현재 WatchTower SQLite 스키마·GitHub API 수집·Producer 연동을 한 공통 Core로 이전. **동일 DB의 수집/갱신 소유자는 반드시 단 하나.** 파일 잠금·인스턴스 리스/소유권/정지-시작 시 인계와 원자적 롤백을 설계·검증한 후 활성화.
3. 실행기: (a) 기존 Tauri에 내장한 호환 모드, (b) Autopilot의 OS 자식 프로세스/CLI로 실행하는 Headless Core. 실행기 없이 Core 라이브러리만 사용하는 것도 가능하지만 Node ↔ Rust 직접 FFI/N-API 결합을 필수로 하지 않음.
4. 인터페이스: 기존 `/v1` 호환 조회 유지. Headless 실행기는 로컬 동일 사용자·인증된 조회 어댑터로 노출; **관리/변경 API는 별도 권한/명령 채널**에서 명시적으로 설계. Node에서 프로세스 수명 관리, 충돌/죽음 감지, 재연결 및 bounded retry를 담당.
5. BuildMap: 개발 PC에서 승인한 **별도 서버 중계 계층** 또는 명시적 export/import. HTTPS + 별도 사용자/프로젝트 인증·최소 권한·데이터 최소화·내역 식별자를 설계하기 전에는 로컬 브리지/PAT를 외부에 공개하지 않음.

```text
                       GitHub Actions / PR
                               |
                 [Rust CI Core: 수집·정규화·귀속·SQLite]
                          단일 쓰기 소유자
                         /               \
              [Tauri 관리 UI]       [Headless 실행기]
                  (선택)             (Autopilot 시작/중지)
                         \               /
                      [인증된 로컬 Read API]
                       /                 \
          [Autopilot GitHub verifier]    [승인된 BuildMap 중계]
               실행 승인 별도           결정/근거 링크
```

**핵심:** 목표는 WatchTower UI 비의존이지, 24시간 상주 서비스 강제가 아님. Autopilot이 실행되는 동안 Core의 라이브러리 또는 자식 실행기를 소유하여 로컬 CI를 관측하고, WatchTower UI를 켤 필요가 없어야 함.

## 3. 기술 방식 비교

| 방식 | 프로세스/배포 | 재사용 및 운영 | 판단 |
|---|---|---|---|
| Rust 공통 Core + 선택적 Headless 실행기 | Autopilot 실행 시 자식 프로세스 1개 가능. Rust 빌드/서명·Windows 배포 필요 | 기존 Rust 코드 재사용 높음, 단일 SQLite writer·토큰 관리 분리 가능, BuildMap 어댑터 재사용 | **권장** |
| Rust 정적/동적 라이브러리 + Node N-API | Autopilot Node 프로세스 1개 | 파일 잠금/크래시 격리 어려움, Node ABI·Windows 바이너리 업데이트/배포 복잡. 나중에 검토 | 보류 |
| Core를 Node.js로 재구현 | 프로세스 1개 | Rust attribution·producer·SQLite 규칙 중복, 이력/정합성 drift 위험 높음 | 비권장 |
| 상주 Rust 서비스 전용 | 독립 프로세스 항상 1개 | 다중 소비자는 편하지만 사용자가 원치 않는 상주·설치 권한·포트/수명 관리 부담 | 선택적 배포 모드 |
| BuildMap에서 GitHub 직접 수집 | 클라우드 수집기 별도 | 사용자 PAT/로컬 Registry를 공유하기 어려우며 수집 중복·Rate Limit·관측 불일치 | 공통 Core 대안 아님 |

우선 라이브러리 경계를 분리하여 기존 앱이 그대로 의존하게 하고, **검증 전 프로세스 소유권은 변경하지 않는다**.

## 4. API 및 실행 계약

- 기존 `/v1/health`, `/v1/registry`, `/v1/runs`, `/v1/runs/{run_id}` 및 PR #66의 `/v1/runs/{run_id}/jobs`는 호출 규약 유지. 새 공통 Core 전환 중 회귀/필드 의미 변화가 있으면 **`/v2`** 제공 및 소비자 명시적 opt-in.
- 불변 최소 필드: `contractVersion`, `source`/관측 출처, `observedAt`, `repositoryId`/이름, `projectId`, `workTrackKey`, `responsibilityKind`, `technicalResponsibility`, `resolutionStatus`, `assignmentSource`, `runId`, `runAttempt`, `latestKnownAttempt`, `headSha`, PR identity·candidate SHA(가능한 경우), `ciKind`, `ciState`, `decisionState`, `evidenceState`, `lastSeenAt`/마지막 성공 poll 시각, Job/Step 원시 상태 및 예외 코드.
- `safeToAdvance=false` 유지. `completeCoverage=false`는 전체 Required Checks 검증이 아니라 표본이라는 뜻. GitHub 현재 PR HEAD·Required Checks·merge/승인 검증은 **Autopilot의 별도 판단**.
- UNKNOWN/STALE/CONFLICT/POLL_ERROR/미등록·비활성 Track/다른 저장소/옛 Attempt·HEAD/권한 실패는 결코 GREEN으로 변경하지 않음.
- 관리 API는 조회 토큰 재사용 금지. Project/Track 수정, PAT 등록, Writer 시작/중지, 수집 강제 재시도에 별도 인증/승인 정책이 필요.
- 로컬 HTTP는 `127.0.0.1` 전용, Bearer Credential Manager 기반, Origin/Host 검증, no-store. BuildMap 원격에서는 PAT나 로컬 Bridge 토큰을 전달하지 않고 별도 user-authenticated API/서명된 export contract를 사용.
- 토큰·Credential 실패는 응답 본문/로그에 포함하지 않음. read-only/관리 권한 분리 및 로그에서 token redaction 강제.

## 5. 데이터/보안 마이그레이션

1. 이전 앱 DB와 OS Credential의 **변경 전 백업** 및 레코드 수·고유 키·이력 checksum 확보; 사용자 PC 기존 백업을 덮어쓰지 않음.
2. Phase 1에서는 **DB 구조/경로/쓰기 주체/스케줄러/이동 없음**. Rust 순수 판정만 독립 crate에서 import.
3. Headless 시험은 **복제한 합성 DB**를 별도 앱 데이터 디렉터리에서 사용; 같은 실사용 DB를 두 Writer가 동시에 열지 않음.
4. 운영 전환 전 기존 WatchTower poller를 정지하고, 독점 writer lease 획득 → schema version 확인 → 기존 데이터 snapshot 검증 → headless 소유권 부여. 실패 시 기존 앱으로 되돌리고 중복 폴링하지 않음.
5. Registry/Track IDs, Alias, Run/Attempt, 원본 PR 연계·수동 귀속, Producer snapshot, notification dedupe를 모두 보존. 과거 자동 귀속을 새 코드로 재판정할 경우 **원본 이력은 불변**, 비교 결과/후보만 별도 기록하고 승인을 거쳐 적용.
6. GitHub PAT는 기존 Windows Credential Manager account를 그대로 유지하되, Core 실행 주체가 동일 OS 사용자라는 것을 검증. 브라우저·BuildMap·저장소·HTTP 응답·Node 로그에 노출 금지.
7. 데이터 복사/전환 실패, writer 중복 감지, 계정 불일치, credential 읽기 실패는 fail-closed로 복구. 다른 PC/클라우드는 별도 사용자 등록·승인 필요.

## 6. 단계 및 완료 기준

| 단계 | 범위 | 완료 증거 |
|---|---|---|
| E0 | 실제 main/미병합 PR/DB/API/Tauri 의존 분석, 이 계획 확정 | 문서, 기존 CI HEAD 및 변경 경계 |
| E1 (이번 PR) | 상태·최신성·Workflow/PR 후보 판정의 **Tauri 독립 Rust crate** 추출, 기존 Bridge가 이를 참조 | 기존 계약 동일, crate 독립 테스트, Windows 기존 Rust/프런트/Installer CI |
| E2 | GitHub 클라이언트·수집 및 Producer/귀속 규칙을 Tauri 비종속 서비스로 이동. DB writer 소유권 명문화 | 합성 DB/이력 회귀, duplicate poll 0, API 오류/취소/재실행 |
| E3 | 선택적 Headless 실행기 및 인증 read-only API, 실행/중지/재시작·프로세스 복구 | UI 완전 종료 중 Autopilot 조회 가능, Windows Credential/PAT 안전, 이전 DB 보존 |
| E4 | WatchTower UI를 Core 소비자로 변경하고 기존 Tauri 명령/조회 동등성 확인 | 기존 UI 기능/Run/Track/알림 회귀, installer 및 롤백 |
| E5 | Autopilot Node에서 Core 실행기/연결 관리, GitHub 별도 승인 게이트 유지 | 단독 실행 E2E, PR/HEAD/Attempt 교차검증, 필수 검사, 무단 진행 0건 |
| E6 | BuildMap 대상 read-only 증거 계약 및 승인된 gateway/export | 원격 개인정보/credential 미노출, provenance 및 결정 링크 정확성, tenant isolation |
| E7 | 기능 플래그로 전환, 기존 브리지 제거 여부 최종 결정 | 실사용 DB 전후 비교, Rate Limit/중복 poll/재시작/Windows 인스톨·롤백 완료 |

**테스트 필수:** 무인증·만료·권한부족/429, network failure, queued/running/success/failure/cancel, head drift, 옛 attempt, PR 충돌, 스테일/미귀속, 다중 소비자, DB 잠금/복구, 재실행/중복 알림 방지, 실제 Actions와 fixture 구분. UI 종료 상태에서의 Autopilot 조회는 **E3 이전에 PASS 주장 금지**.

## 7. 외부 저장소 후속 작업

- **Autopilot:** 현재 단일 정확 Run/Attempt 바인딩과 #81/#82/#83 의존관계를 보존하면서 Headless Core 라이프사이클/health/restart 로직 추가. 기존 `/v1` API가 유지되는 동안 이전 클라이언트 경로도 유지. Telegram/승인 정책·GitHub PR 검증은 기존 Autopilot 책임.
- **BuildMap:** 단순 Rust localhost 주소를 웹앱에 삽입하지 않고, 사용자별 CI evidence/decision linkage 모델과 별도 인증된 import/export 또는 backend proxy를 설계. 이미 있는 GitHub 관측 기능과 수집 충돌/중복 여부 확인.
- **WatchTower:** #66 → #67 → #68 스택의 명시적 운영 승인·병합 전에는 이번 변경도 별도 stacked PR. 사용자 Windows 0.3.40의 프로젝트 2/Repository 3/Track 22 및 백업 보존을 외부 실환경 Acceptance에서 확인해야 함.

## 8. 즉시 중단·승인 조건

- 실사용 데이터 DB 경로 이전/쓰기·자격 증명 재발급·기존 브리지 종료·무중단 cutover는 운영자 승인과 실제 Windows 검증 전 금지.
- 열려 있는 #66/#67/#68 또는 Autopilot 종속 PR을 임의 병합/배포 금지.
- 설계와 E1 소스 분리는 승인 불필요한 개발 브랜치에서만 진행.
