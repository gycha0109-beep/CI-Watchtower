import { useCallback, useEffect, useMemo, useState, type FormEvent } from 'react';
import { api } from './api';
import type {
  Dashboard,
  DashboardTrack,
  MonitoredRepository,
  Project,
  Settings,
  TrackInput,
  WorkflowRunSummary,
} from './types';

type TrackFormState = Omit<TrackInput, 'longCiMinutes'> & { longCiMinutes: number | '' };
type ViewFilter = 'all' | 'project' | 'unassigned' | number;
type ScopeFilter = 'all' | number;

const emptyTrack = (projectId = 0): TrackFormState => ({
  projectId,
  name: '',
  trackKey: '',
  longCiMinutes: '',
});

function formatDuration(seconds: number | null | undefined) {
  if (seconds == null) return '—';
  if (seconds < 60) return `${seconds}s`;
  const m = Math.floor(seconds / 60);
  const s = seconds % 60;
  if (m < 60) return `${m}m ${s}s`;
  return `${Math.floor(m / 60)}h ${m % 60}m`;
}

function statusLabel(item: DashboardTrack): [string, string] {
  switch (item.health) {
    case 'green': return ['GREEN', 'status green'];
    case 'red': return ['RED', 'status red'];
    case 'running': return ['RUNNING', 'status running'];
    case 'queued': return ['QUEUED', 'status queued'];
    case 'completed_other': return ['DONE', 'status amber'];
    default: return ['WAITING', 'status muted'];
  }
}

function runDot(run: WorkflowRunSummary) {
  if (run.status === 'completed') return run.conclusion === 'success' ? 'green-dot' : 'red-dot';
  if (['queued', 'requested', 'pending', 'waiting'].includes(run.status)) return 'queued-dot';
  return 'running-dot';
}

function repositoryState(repo: MonitoredRepository) {
  if (!repo.enabled) return 'OFF';
  if (repo.lastError) return 'ERROR';
  if (repo.runningCount > 0) return `RUNNING ${repo.runningCount}`;
  if (repo.queuedCount > 0) return `QUEUED ${repo.queuedCount}`;
  return 'IDLE';
}

function trackActivity(item: DashboardTrack) {
  const running = item.runs.filter(run => run.status === 'in_progress').length;
  const queued = item.runs.filter(run => ['queued', 'requested', 'pending', 'waiting'].includes(run.status)).length;
  const red = item.runs.filter(run =>
    run.status === 'completed' &&
    ['failure', 'cancelled', 'timed_out', 'action_required', 'startup_failure', 'stale'].includes(run.conclusion ?? '')
  ).length;
  return { running, queued, red };
}

function App() {
  const [dashboard, setDashboard] = useState<Dashboard | null>(null);
  const [trackForm, setTrackForm] = useState<TrackFormState>(emptyTrack());
  const [projectName, setProjectName] = useState('');
  const [projectKey, setProjectKey] = useState('');
  const [repoInput, setRepoInput] = useState('');
  const [repoProjectId, setRepoProjectId] = useState(0);
  const [ruleName, setRuleName] = useState('');
  const [ruleRepositoryId, setRuleRepositoryId] = useState<ScopeFilter>('all');
  const [token, setToken] = useState('');
  const [editingId, setEditingId] = useState<number | null>(null);
  const [selectedProject, setSelectedProject] = useState<ScopeFilter>('all');
  const [selectedRepository, setSelectedRepository] = useState<ScopeFilter>('all');
  const [selectedView, setSelectedView] = useState<ViewFilter>('all');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  const refresh = useCallback(async (poll = false) => {
    try {
      setBusy(true);
      setError(null);
      setDashboard(await (poll ? api.pollNow() : api.getDashboard()));
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }, []);

  useEffect(() => {
    void refresh(false);
    const id = window.setInterval(() => void refresh(false), 5000);
    return () => window.clearInterval(id);
  }, [refresh]);

  useEffect(() => {
    if (!dashboard || dashboard.projects.length === 0) return;
    const first = dashboard.projects[0].id;
    if (trackForm.projectId === 0) setTrackForm(emptyTrack(first));
    if (repoProjectId === 0) setRepoProjectId(first);
    if (selectedProject === 'all' && dashboard.projects.length === 1) setSelectedProject(first);
  }, [dashboard, repoProjectId, selectedProject, trackForm.projectId]);

  useEffect(() => {
    if (!dashboard) return;
    if (typeof selectedView === 'number' && !dashboard.tracks.some(item => item.track.id === selectedView)) setSelectedView('all');
    if (typeof selectedProject === 'number' && !dashboard.projects.some(project => project.id === selectedProject)) setSelectedProject('all');
    if (typeof selectedRepository === 'number' && !dashboard.repositories.some(repo => repo.id === selectedRepository)) setSelectedRepository('all');
  }, [dashboard, selectedProject, selectedRepository, selectedView]);

  const projectById = useMemo(
    () => new Map((dashboard?.projects ?? []).map(project => [project.id, project])),
    [dashboard],
  );

  const repositoriesInScope = useMemo(() => {
    if (!dashboard) return [];
    return dashboard.repositories.filter(repo =>
      (selectedProject === 'all' || repo.projectId === selectedProject) &&
      (selectedRepository === 'all' || repo.id === selectedRepository)
    );
  }, [dashboard, selectedProject, selectedRepository]);

  const tracksInProject = useMemo(() => {
    if (!dashboard) return [];
    return dashboard.tracks.filter(item => selectedProject === 'all' || item.track.projectId === selectedProject);
  }, [dashboard, selectedProject]);

  const runInScope = useCallback((run: WorkflowRunSummary) => (
    (selectedProject === 'all' || run.projectId === selectedProject) &&
    (selectedRepository === 'all' || run.repositoryId === selectedRepository)
  ), [selectedProject, selectedRepository]);

  const visibleTracks = useMemo(() => {
    if (selectedView === 'project' || selectedView === 'unassigned') return [];
    const base = selectedView === 'all'
      ? tracksInProject
      : tracksInProject.filter(item => item.track.id === selectedView);
    return base.map(item => ({ ...item, runs: item.runs.filter(runInScope) }));
  }, [runInScope, selectedView, tracksInProject]);

  const visibleProjectRuns = useMemo(
    () => (dashboard?.projectRuns ?? []).filter(runInScope),
    [dashboard, runInScope],
  );

  const visibleUnassignedRuns = useMemo(
    () => (dashboard?.unassignedRuns ?? []).filter(runInScope),
    [dashboard, runInScope],
  );

  const runningCount = repositoriesInScope.reduce((sum, repo) => sum + repo.runningCount, 0);
  const queuedCount = repositoriesInScope.reduce((sum, repo) => sum + repo.queuedCount, 0);
  const congestionText = !dashboard
    ? '—'
    : queuedCount >= dashboard.settings.queueCongestionThreshold
      ? 'CONGESTED'
      : queuedCount >= Math.ceil(dashboard.settings.queueCongestionThreshold / 2)
        ? 'BUSY'
        : 'SAFE';

  const selectedProjectId = typeof selectedProject === 'number'
    ? selectedProject
    : dashboard?.projects[0]?.id ?? 0;

  const selectedProjectRepos = useMemo(
    () => (dashboard?.repositories ?? []).filter(repo => selectedProject === 'all' || repo.projectId === selectedProject),
    [dashboard, selectedProject],
  );

  const act = async (work: () => Promise<unknown>, message?: string, poll = false) => {
    try {
      setError(null);
      setNotice(null);
      await work();
      if (message) setNotice(message);
      await refresh(poll);
    } catch (e) {
      setError(String(e));
    }
  };

  const submitProject = async (e: FormEvent) => {
    e.preventDefault();
    const name = projectName.trim();
    const key = projectKey.trim().toLowerCase();
    if (!name || !key) return;
    await act(async () => {
      const id = await api.saveProject({ name, projectKey: key });
      setProjectName('');
      setProjectKey('');
      setSelectedProject(id);
      setSelectedRepository('all');
      setSelectedView('all');
      setTrackForm(emptyTrack(id));
      setRepoProjectId(id);
    }, '프로젝트를 추가했습니다.');
  };

  const submitTrack = async (e: FormEvent) => {
    e.preventDefault();
    const payload: TrackInput = {
      id: editingId ?? undefined,
      projectId: trackForm.projectId,
      name: trackForm.name.trim(),
      trackKey: trackForm.trackKey.trim().toLowerCase(),
      longCiMinutes: Number(trackForm.longCiMinutes),
    };
    if (!payload.projectId || !payload.name || !payload.trackKey || payload.longCiMinutes <= 0) return;
    await act(async () => {
      const savedId = await api.saveTrack(payload);
      setTrackForm(emptyTrack(payload.projectId));
      setEditingId(null);
      setSelectedProject(payload.projectId);
      setSelectedView(savedId);
    }, editingId ? '트랙을 수정했습니다.' : '트랙을 추가했습니다.', true);
  };

  const submitRepository = async (e: FormEvent) => {
    e.preventDefault();
    const repo = repoInput.trim();
    if (!repo || !repoProjectId) return;
    await act(async () => {
      await api.saveRepository({ projectId: repoProjectId, repo, enabled: true });
      setRepoInput('');
      setSelectedProject(repoProjectId);
    }, '감시 저장소를 추가했습니다.', true);
  };

  const submitRule = async (e: FormEvent) => {
    e.preventDefault();
    if (!ruleName.trim() || !selectedProjectId) return;
    await act(async () => {
      await api.saveProjectWorkflowRule({
        projectId: selectedProjectId,
        repositoryId: ruleRepositoryId === 'all' ? null : ruleRepositoryId,
        workflowName: ruleName.trim(),
      });
      setRuleName('');
      setRuleRepositoryId('all');
      setSelectedProject(selectedProjectId);
      setSelectedView('project');
    }, '공용 CI 규칙을 추가했습니다.', true);
  };

  const editTrack = (item: DashboardTrack) => {
    setEditingId(item.track.id);
    setTrackForm({
      id: item.track.id,
      projectId: item.track.projectId,
      name: item.track.name,
      trackKey: item.track.trackKey,
      longCiMinutes: item.track.longCiMinutes,
    });
    setSelectedProject(item.track.projectId);
    setSelectedView(item.track.id);
    document.querySelector('.controls-panel')?.scrollTo({ top: 0, behavior: 'smooth' });
  };

  const removeTrack = async (item: DashboardTrack) => {
    if (!window.confirm(`"${item.track.name}" 트랙을 삭제하시겠습니까?`)) return;
    await act(async () => {
      await api.deleteTrack(item.track.id);
      if (editingId === item.track.id) {
        setEditingId(null);
        setTrackForm(emptyTrack(item.track.projectId));
      }
      if (selectedView === item.track.id) setSelectedView('all');
    }, `${item.track.name} 트랙을 삭제했습니다.`);
  };

  const removeProject = async (project: Project) => {
    if (!window.confirm(`"${project.name}" 프로젝트를 삭제하시겠습니까? 저장소/트랙이 남아 있으면 삭제되지 않습니다.`)) return;
    await act(() => api.deleteProject(project.id), `${project.name} 프로젝트를 삭제했습니다.`);
  };

  const saveToken = async (e: FormEvent) => {
    e.preventDefault();
    if (!token.trim()) return;
    await act(async () => {
      await api.setGithubToken(token.trim());
      setToken('');
    }, 'PAT 저장 및 재조회 검증을 완료했습니다.', true);
  };

  const updateSettings = async (patch: Partial<Settings>) => {
    if (!dashboard) return;
    const next = { ...dashboard.settings, ...patch };
    await act(() => api.saveSettings(next));
  };

  const selectProject = (project: ScopeFilter) => {
    setSelectedProject(project);
    setSelectedRepository('all');
    setSelectedView('all');
    if (typeof project === 'number') {
      setTrackForm(current => ({ ...current, projectId: project }));
      setRepoProjectId(project);
    }
  };

  return (
    <main className="app-shell">
      <header className="topbar">
        <div>
          <p className="eyebrow">GITHUB ACTIONS CONTROL · V0.3</p>
          <h1>CI Watchtower</h1>
          <p className="subtitle">Project → Track → Repository 계층으로 Actions Run을 분류합니다.</p>
        </div>
        <button className="primary" onClick={() => void refresh(true)} disabled={busy}>
          {busy ? '확인 중…' : '지금 확인'}
        </button>
      </header>

      {error && <div className="error-banner">{error}</div>}
      {notice && <div className="notice-banner">{notice}</div>}

      <section className="panel project-switcher">
        <div className="section-head compact-head">
          <div><p className="eyebrow">PROJECT</p><h2>프로젝트</h2></div>
          <span className="muted-copy">저장소가 여러 개여도 하나의 제품/프로젝트 단위로 묶습니다.</span>
        </div>
        <div className="filter-chips">
          <button className={`filter-chip ${selectedProject === 'all' ? 'active' : ''}`} onClick={() => selectProject('all')}>
            전체 프로젝트 <span>{dashboard?.projects.length ?? 0}</span>
          </button>
          {dashboard?.projects.map(project => (
            <button key={project.id} className={`filter-chip ${selectedProject === project.id ? 'active' : ''}`} onClick={() => selectProject(project.id)}>
              {project.name}
            </button>
          ))}
        </div>
      </section>

      <section className="summary-grid five">
        <div className="summary-card"><span>Running</span><strong>{runningCount}</strong></div>
        <div className="summary-card"><span>Queued</span><strong>{queuedCount}</strong></div>
        <button className="summary-card summary-button" onClick={() => setSelectedView('unassigned')}>
          <span>Unassigned</span><strong>{dashboard?.unassignedCount ?? 0}</strong>
        </button>
        <button className="summary-card summary-button" onClick={() => setSelectedView('project')}>
          <span>Project CI</span><strong>{visibleProjectRuns.length}</strong>
        </button>
        <div className={`summary-card congestion ${congestionText.toLowerCase()}`}><span>Queue</span><strong>{congestionText}</strong></div>
      </section>

      <div className="layout-grid">
        <section className="panel controls-panel">
          <h2>프로젝트 등록</h2>
          <form onSubmit={submitProject} className="stack-form">
            <label>프로젝트 이름<input value={projectName} onChange={e => setProjectName(e.target.value)} placeholder="예: 비주얼리" /></label>
            <label>Project Key<input value={projectKey} onChange={e => setProjectKey(e.target.value)} placeholder="visualy" /></label>
            <button className="primary" type="submit">프로젝트 추가</button>
          </form>
          <div className="project-manager-list">
            {dashboard?.projects.map(project => (
              <div className="project-manager-item" key={project.id}>
                <button className="managed-track-main" onClick={() => selectProject(project.id)}>
                  <span className="managed-track-copy"><b>{project.name}</b><small className="mono">{project.projectKey}</small></span>
                </button>
                <button className="danger-ghost tiny" onClick={() => void removeProject(project)}>삭제</button>
              </div>
            ))}
          </div>

          <hr />
          <h2>{editingId ? '트랙 수정' : '트랙 등록'}</h2>
          <form onSubmit={submitTrack} className="stack-form">
            <label>프로젝트
              <select value={trackForm.projectId} onChange={e => setTrackForm({ ...trackForm, projectId: Number(e.target.value) })}>
                {dashboard?.projects.map(project => <option key={project.id} value={project.id}>{project.name}</option>)}
              </select>
            </label>
            <label>트랙 이름<input value={trackForm.name} onChange={e => setTrackForm({ ...trackForm, name: e.target.value })} placeholder="예: 프론트 연동" /></label>
            <label>Track Key<input value={trackForm.trackKey} onChange={e => setTrackForm({ ...trackForm, trackKey: e.target.value })} placeholder="frontend-integration" /></label>
            <label>장기 CI 기준시간<input type="number" min="1" step="1" value={trackForm.longCiMinutes} onChange={e => setTrackForm({ ...trackForm, longCiMinutes: e.target.value === '' ? '' : Number(e.target.value) })} placeholder="8" /></label>
            <div className="row-actions">
              <button className="primary" type="submit">{editingId ? '수정 저장' : '트랙 추가'}</button>
              {editingId && <button type="button" className="ghost" onClick={() => { setEditingId(null); setTrackForm(emptyTrack(trackForm.projectId)); }}>취소</button>}
            </div>
          </form>

          <div className="sidebar-section track-manager">
            <div className="sidebar-section-head"><h2>등록된 트랙</h2><span>{tracksInProject.length}</span></div>
            <div className="managed-track-list">
              {tracksInProject.map(item => {
                const [label, cls] = statusLabel(item);
                return (
                  <div className={`managed-track-item ${selectedView === item.track.id ? 'selected' : ''}`} key={item.track.id}>
                    <button className="managed-track-main" onClick={() => { setSelectedProject(item.track.projectId); setSelectedView(item.track.id); }}>
                      <span className={cls}>{label}</span>
                      <span className="managed-track-copy"><b>{item.track.name}</b><small className="mono">{item.track.trackKey} · {item.track.longCiMinutes}m</small></span>
                    </button>
                    <div className="managed-track-actions">
                      <button className="ghost tiny" onClick={() => editTrack(item)}>수정</button>
                      <button className="danger-ghost tiny" onClick={() => void removeTrack(item)}>삭제</button>
                    </div>
                  </div>
                );
              })}
            </div>
          </div>

          <hr />
          <h2>감시 저장소</h2>
          <form onSubmit={submitRepository} className="stack-form">
            <label>프로젝트
              <select value={repoProjectId} onChange={e => setRepoProjectId(Number(e.target.value))}>
                {dashboard?.projects.map(project => <option key={project.id} value={project.id}>{project.name}</option>)}
              </select>
            </label>
            <label>Repository<input value={repoInput} onChange={e => setRepoInput(e.target.value)} placeholder="gycha0109-beep/MyeongHa" /></label>
            <button className="primary" type="submit">저장소 추가</button>
          </form>
          <div className="repo-list">
            {dashboard?.repositories.filter(repo => selectedProject === 'all' || repo.projectId === selectedProject).map(repo => (
              <div className="repo-item" key={repo.id}>
                <div><b>{repo.repo}</b><span>{projectById.get(repo.projectId)?.name ?? '—'} · <span className={repo.lastError ? 'repo-error' : ''}>{repositoryState(repo)}</span></span></div>
                <div className="row-actions">
                  <button className="ghost small" onClick={() => void act(() => api.saveRepository({ id: repo.id, projectId: repo.projectId, repo: repo.repo, enabled: !repo.enabled }), undefined, true)}>{repo.enabled ? '중지' : '감시'}</button>
                  <button className="danger-ghost small" onClick={() => void act(() => api.deleteRepository(repo.id))}>삭제</button>
                </div>
              </div>
            ))}
          </div>

          <hr />
          <h2>공용 CI 규칙</h2>
          <p className="hint">특정 트랙이 아니라 프로젝트 전체를 검증하는 Workflow 이름을 등록합니다.</p>
          <form onSubmit={submitRule} className="stack-form">
            <label>Workflow 이름<input value={ruleName} onChange={e => setRuleName(e.target.value)} placeholder="예: Governance" /></label>
            <label>저장소 범위
              <select value={ruleRepositoryId} onChange={e => setRuleRepositoryId(e.target.value === 'all' ? 'all' : Number(e.target.value))}>
                <option value="all">프로젝트 전체</option>
                {selectedProjectRepos.map(repo => <option key={repo.id} value={repo.id}>{repo.repo}</option>)}
              </select>
            </label>
            <button className="primary" type="submit" disabled={!selectedProjectId}>공용 규칙 추가</button>
          </form>
          <div className="rule-list">
            {dashboard?.projectWorkflowRules.filter(rule => selectedProject === 'all' || rule.projectId === selectedProject).map(rule => (
              <div className="rule-item" key={rule.id}>
                <div><b>{rule.workflowName}</b><span>{rule.repositoryId ? dashboard.repositories.find(repo => repo.id === rule.repositoryId)?.repo : '프로젝트 전체'}</span></div>
                <button className="danger-ghost tiny" onClick={() => void act(() => api.deleteProjectWorkflowRule(rule.id), '공용 CI 규칙을 삭제했습니다.', true)}>삭제</button>
              </div>
            ))}
          </div>

          <hr />
          <h2>GitHub 인증</h2>
          <form onSubmit={saveToken} className="stack-form">
            <label>Fine-grained PAT<input type="password" value={token} onChange={e => setToken(e.target.value)} placeholder="github_pat_..." autoComplete="off" /></label>
            <div className="row-actions">
              <button className="primary" type="submit">PAT 저장</button>
              {dashboard?.tokenConfigured && <button type="button" className="danger-ghost" onClick={() => void act(() => api.clearGithubToken())}>삭제</button>}
            </div>
          </form>

          <hr />
          <h2>감시 설정</h2>
          {dashboard && <div className="stack-form">
            <label>Queue 혼잡 경고 기준<input type="number" min="1" value={dashboard.settings.queueCongestionThreshold} onBlur={e => void updateSettings({ queueCongestionThreshold: Number(e.target.value) })} onChange={e => setDashboard({ ...dashboard, settings: { ...dashboard.settings, queueCongestionThreshold: Number(e.target.value) } })} /></label>
            <label>활성 Polling(초)<input type="number" min="10" value={dashboard.settings.activePollSeconds} onBlur={e => void updateSettings({ activePollSeconds: Number(e.target.value) })} onChange={e => setDashboard({ ...dashboard, settings: { ...dashboard.settings, activePollSeconds: Number(e.target.value) } })} /></label>
            <label>유휴 Polling(초)<input type="number" min="30" value={dashboard.settings.idlePollSeconds} onBlur={e => void updateSettings({ idlePollSeconds: Number(e.target.value) })} onChange={e => setDashboard({ ...dashboard, settings: { ...dashboard.settings, idlePollSeconds: Number(e.target.value) } })} /></label>
          </div>}
        </section>

        <section className="tracks-column">
          <section className="panel track-filter-panel">
            <div className="filter-head">
              <div><p className="eyebrow">SCOPE</p><h2>{typeof selectedProject === 'number' ? projectById.get(selectedProject)?.name : '전체 프로젝트'}</h2></div>
              <select className="repo-filter" value={selectedRepository} onChange={e => setSelectedRepository(e.target.value === 'all' ? 'all' : Number(e.target.value))}>
                <option value="all">Repository: 전체</option>
                {selectedProjectRepos.map(repo => <option key={repo.id} value={repo.id}>{repo.repo}</option>)}
              </select>
            </div>
            <div className="filter-chips">
              <button className={`filter-chip ${selectedView === 'all' ? 'active' : ''}`} onClick={() => setSelectedView('all')}>전체 <span>{tracksInProject.length}</span></button>
              <button className={`filter-chip project-wide ${selectedView === 'project' ? 'active' : ''}`} onClick={() => setSelectedView('project')}>공용 CI <span>{visibleProjectRuns.length}</span></button>
              {tracksInProject.map(item => {
                const activity = trackActivity(item);
                const activeCount = activity.running + activity.queued;
                return (
                  <button key={item.track.id} className={`filter-chip ${selectedView === item.track.id ? 'active' : ''}`} onClick={() => setSelectedView(item.track.id)}>
                    {item.track.name}
                    {activeCount > 0 && <span className="chip-live">{activeCount}</span>}
                    {activeCount === 0 && activity.red > 0 && <span className="chip-red">RED</span>}
                  </button>
                );
              })}
              <button className={`filter-chip unassigned ${selectedView === 'unassigned' ? 'active' : ''}`} onClick={() => setSelectedView('unassigned')}>미귀속 <span>{visibleUnassignedRuns.length}</span></button>
            </div>
          </section>

          {selectedView === 'project' && (
            <section className="panel inbox-panel">
              <div className="section-head"><div><p className="eyebrow">PROJECT-WIDE CI</p><h2>공용 CI</h2></div><strong>{visibleProjectRuns.length}</strong></div>
              {visibleProjectRuns.length === 0 ? <p className="muted-copy">현재 범위에 공용 CI가 없습니다.</p> : visibleProjectRuns.map(run => (
                <button className="run-row project-run-row" key={run.id} onClick={() => void api.openExternal(run.htmlUrl)}>
                  <span className={`dot ${runDot(run)}`} />
                  <span className="run-main"><b>{run.workflowName}</b><small>{projectById.get(run.projectId)?.name} · {run.repository} · {run.headBranch ?? 'detached'} · <span className="mono">{run.headSha.slice(0, 8)}</span></small></span>
                  <span className="run-state">{run.status}{run.conclusion ? ` · ${run.conclusion}` : ''}</span><span>{formatDuration(run.elapsedSeconds)}</span>
                </button>
              ))}
            </section>
          )}

          {selectedView === 'unassigned' && (
            <section className="panel inbox-panel">
              <div className="section-head">
                <div><p className="eyebrow">ATTRIBUTION INBOX</p><h2>미귀속 CI</h2><p className="muted-copy">현재 범위 {visibleUnassignedRuns.length}건 · 전체 DB {dashboard?.unassignedCount ?? 0}건 · 최대 최근 200건 표시</p></div>
              </div>
              {visibleUnassignedRuns.length === 0 ? <p className="muted-copy">현재 확인이 필요한 미귀속 CI가 없습니다.</p> : visibleUnassignedRuns.map(run => (
                <div className="unassigned-run" key={run.id}>
                  <div className="unassigned-main">
                    <span className={`dot ${runDot(run)}`} />
                    <div><b>{run.workflowName}</b><p>{projectById.get(run.projectId)?.name} · {run.repository} · {run.event} · {run.headBranch ?? 'detached'} · <span className="mono">{run.headSha.slice(0, 8)}</span></p><p className="reason">{run.resolutionStatus === 'conflict' ? `TRACK CONFLICT · ${run.attributionReason ?? '명시 신호 충돌'}` : (run.attributionReason ?? '명시적 Track Key를 찾지 못했습니다.')}</p></div>
                  </div>
                  <div className="assignment-actions">
                    {(dashboard?.tracks ?? []).filter(item => item.track.projectId === run.projectId).map(item => (
                      <button key={item.track.id} className="ghost small" onClick={() => void act(() => api.assignRun(run.id, item.track.id), `${item.track.name}에 귀속했습니다.`)}>{item.track.name}</button>
                    ))}
                    <button className="ghost small project-action" onClick={() => void act(() => api.assignRunToProject(run.id, run.projectId, true), '동일 Workflow를 프로젝트 공용 CI로 분류했습니다.', true)}>공용 CI로 분류</button>
                    <button className="danger-ghost small" onClick={() => void act(() => api.ignoreRun(run.id))}>무시</button>
                    <button className="ghost small" onClick={() => void api.openExternal(run.htmlUrl)}>Actions</button>
                  </div>
                </div>
              ))}
            </section>
          )}

          {selectedView !== 'unassigned' && selectedView !== 'project' && visibleTracks.length === 0 && (
            <div className="panel empty-state"><h2>표시할 트랙이 없습니다.</h2><p>선택한 프로젝트에 Track Key를 등록하십시오.</p></div>
          )}

          {visibleTracks.map(item => {
            const [label, cls] = statusLabel(item);
            return (
              <article className="panel track-card" key={item.track.id}>
                <div className="track-head">
                  <div><div className={cls}>{label}</div><h3>{item.track.name}</h3><p className="mono">{projectById.get(item.track.projectId)?.name} / {item.track.trackKey}</p></div>
                  <div className="track-actions"><button className="icon-btn" onClick={() => editTrack(item)}>수정</button><button className="icon-btn danger" onClick={() => void removeTrack(item)}>삭제</button></div>
                </div>
                <div className="metrics-row three">
                  <div><span>현재 경과</span><b>{formatDuration(item.elapsedSeconds)}</b></div>
                  <div><span>최근 20회 평균</span><b>{formatDuration(item.averageDurationSeconds)}</b></div>
                  <div><span>장기 기준</span><b>{item.track.longCiMinutes}m</b></div>
                </div>
                <div className="runs-list">
                  {item.runs.length === 0 ? <p className="muted-copy">현재 Repository 필터에 표시할 Run이 없습니다.</p> : item.runs.map(run => (
                    <button className="run-row v2" key={run.id} onClick={() => void api.openExternal(run.htmlUrl)}>
                      <span className={`dot ${runDot(run)}`} />
                      <span className="run-main"><b>{run.workflowName}</b><small>{run.repository} · {run.headBranch ?? 'detached'} · <span className="mono">{run.headSha.slice(0, 8)}</span></small></span>
                      <span className="run-state">{run.status}{run.conclusion ? ` · ${run.conclusion}` : ''}</span><span>{formatDuration(run.elapsedSeconds)}</span><span className="attribution">{run.attributionSource ?? 'resolver'} · {run.confidence ?? 0}</span>
                    </button>
                  ))}
                </div>
              </article>
            );
          })}
        </section>
      </div>
    </main>
  );
}

export default App;
