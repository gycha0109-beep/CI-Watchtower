# CI Core — UI 없는 최신 GitHub Actions 실행 목록 조회

`ci-core-scan.exe`는 기존 WatchTower의 GitHub PAT를 **같은 Windows 사용자 Credential Manager**에서 읽고, 명시한 저장소의 최근 Actions 실행을 일회성으로 조회합니다. WatchTower 앱이나 HTTP 브리지를 실행할 필요가 없습니다.

```powershell
.\ci-core-scan.exe --repository owner/repo --limit 30
```

실행당 GitHub GET은 **최대 6회**로 제한합니다. 최근 실행 100개 한 페이지와 상태별(queued/in_progress/waiting/pending/requested) 한 페이지씩만 확인합니다. 따라서 100건을 넘는 상태별 실행과 전 페이지를 전부 확인했다고 주장하지 않습니다. 동일 Run ID 중 최신 Attempt를 보존하고 시간 순으로 최대 50개 반환합니다.

결과 `ci-core-scan/0.1`에는 `repository`, `observedAt`, `apiRequests`, `sampleLimit`, `runs`(Run ID, workflow, Head SHA, Attempt, 상태, PR 번호)와 `completeCoverage=false`, `targetVerified=false`, `requiredChecksVerified=false`, `safeToAdvance=false`가 있습니다.

조회 결과는 **참고용 실시간 목록**입니다. GitHub의 특정 Run/PR/Attempt 동일성 검증, Project/Track 귀속 검증, required checks, merged 확인, 사용자 승인, supersession 확인은 별개입니다. 특히 과거 SUCCESS나 워크플로 제목만으로 작업을 재개하지 않습니다.

PAT를 명령줄이나 결과로 출력하지 않으며, 오류는 인증/요청 제한/권한/전송/응답 형식 등 고정 코드로만 반환합니다. SQLite 읽기/쓰기나 폴링 스케줄은 이 실행 파일 안에 없습니다.

전체 자동 관제를 위해 다음에 Autopilot의 **최대 2분 간격·제한된 저장소 개수·403 백오프**로 호출하고, 기존 로컬 DB 이력을 참조용으로 병합합니다. 이후 단일 writer 구조에서 실제 Run/Attempt 수집과 트랙 귀속을 이전해야 합니다. 새 독립 실행 파일은 시험용 미배포 PR에만 있습니다.
