# CI Core — 격리 DB에서만 실행하는 Run/Attempt 기록 엔진

독립 실행 파일 `ci-core-stage.exe`는 기존 WatchTower 데이터베이스를 **읽기 전용으로 열고**, SQLite 공식 온라인 백업 API로 새 시험 DB를 생성합니다. 이후 기존 Windows Credential Manager PAT로 GitHub Actions 최신 1페이지(지정한 최대 30건)를 조회하여 **시험 복사본에만** 공통 Run/Attempt 저장 함수를 실행합니다.

이 모드는 **운영 DB 쓰기 전환이 아닙니다.** 기존 WatchTower 프로세스가 꺼져 있더라도 운영 DB의 writer 자격을 가져오지 않습니다.

## Windows 시험 예

```powershell
.\ci-core-stage.exe --source "$env:APPDATA\com.gycha.ciwatchtower\ci-watchtower.sqlite3" --stage "$env:TEMP\ci-core-stage-test-unique.sqlite3" --repository-id 1 --limit 20
```

`--stage`는 **아직 존재하지 않는 절대 경로**여야 하며 파일이 있으면 즉시 거부됩니다. `--repository-id`는 GitHub 저장소 ID가 아니라 기존 SQLite에 저장된 로컬 Repository ID입니다. 입력한 저장소가 비활성 또는 프로젝트 미지정이면 거부됩니다. 프로그램이 원본 경로를 생성·수정·마이그레이션하거나 원본에 접속해 SQL을 기록하지 않습니다.

## 데이터 무결성

- 기존 DB는 `SQLITE_OPEN_READ_ONLY`로 연결합니다. 원본 SQLite/WAL의 일관된 온라인 복제본을 생성하고, 복사본에만 Run/Attempt를 기록합니다.
- WatchTower가 이미 사용하던 `workflow_runs` / `run_attempts` 컬럼, 과거 Attempt, PR 번호, 수동 Track 귀속을 보존합니다.
- 같은 Run/Attempt를 반복 수집하면 덮어쓰기 대신 해당 Attempt를 갱신합니다. 더 오래된 Attempt나 업데이트 시점이 뒤처진 응답은 무시하고, 기존 Run ID가 다른 Repository에 연결되어 있으면 오류 처리합니다.
- Run과 Attempt의 두 저장 명령을 한 SQLite 트랜잭션으로 묶어 중간 실패 때 함께 롤백합니다.
- Github 조회 1회만 수행하며, 실패 시 시험 파일을 생성하지 않습니다. PAT·원문 GitHub 에러·데이터 원본 경로를 결과/오류에 포함하지 않습니다.
- 결과에는 원본/시험 DB의 저장소별 Run 개수, 기록/건너뜀 개수, `stagingOnly=true`, `completeCoverage=false`, `safeToAdvance=false`가 표시됩니다.

## 아직 남은 운영 전환 조건

이 기능이 성공해도 운영 SQLite 단일 writer 전환은 **미완료**입니다. WatchTower 기존 poller와 독립 writer가 동시에 쓰지 않도록 프로세스 수준 잠금/소유권 이전, Producer 및 Track 귀속·이력·알림 중복 방지, 장애 복구, 사용자 승인, 현재 PR/Attempt/필수 검사 검증을 마쳐야 합니다.

실환경 검증 시 반드시 사전/사후 원본 DB SHA-256, `PRAGMA integrity_check`, 프로젝트·저장소·Track 수, 수동 귀속 건수를 대조합니다. 시험 복사본은 검증 후 필요에 따라 별도 폐기하되 운영 DB/백업은 건드리지 않습니다.
