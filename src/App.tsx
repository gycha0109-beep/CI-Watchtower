import { useCallback, useEffect, useMemo, useState, type FormEvent } from 'react';
import { api } from './api';
import type {
  Dashboard,
  DashboardTrack,
  MonitoredRepository,
  Project,
  ResponsibilityMapDrift,
  ResponsibilityResolutionPreview,
  RunAttributionDetail,
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

function producerBucketLabel(bucket: string) {
  switch (bucket) {
    case 'inference': return 'HEURISTIC';
    case 'manual': return 'MANUAL';
    case 'track_alias': return 'ALIAS';
    case 'unassigned': return 'UNASSIGNED';
    case 'conflict': return 'CONFLICT';
    case 'other': return 'OTHER';
    default: return bucket.toUpperCase();
  }
}

function responsibilityDriftLabel(type: string) {
  switch (type) {
    case 'missing_in_watchtower': return 'MISSING';
    case 'stale_in_watchtower': return 'STALE';
    case 'responsibility_kind_mismatch': return 'KIND';
    case 'track_binding_mismatch': return 'TRACK';
    default: return type.toUpperCase();
  }
}

function responsibilityActionLabel(action: string) {
  switch (action) {
    case 'review_add_project_wide_rule': return 'Project-wide 선언 검토';
    case 'review_add_dynamic_rule': return 'Dynamic 선언 검토';
    case 'review_reclassify_project_wide': return 'Project-wide 재분류 검토';
    case 'review_reclassify_dynamic': return 'Dynamic 재분류 검토';
    case 'review_remove_conflicting_responsibility_rule': return '충돌 규칙 제거 검토';
    case 'review_track_registry_or_map': return 'Track / Map 계약 검토';
    case 'review_producer_run_name': return 'Producer run-name 수정 검토';
    case 'review_repository_map_binding': return 'Repository map 계약 검토';
    case 'review_remove_or_confirm_stale_rule': return 'Stale 규칙 유지 / 제거 검토';
    default: return action;
  }
}

function resolutionActionLabel(action: string) {
  switch (action) {
    case 'add_project_wide_rule': return 'Repository-scoped Project-wide 규칙 추가';
    case 'add_dynamic_rule': return 'Repository-scoped protected Dynamic 규칙 추가';
    case 'reclassify_to_project_wide': return 'Dynamic → Project-wide 재분류';
    case 'reclassify_to_dynamic': return 'Project-wide → Dynamic 재분류';
    case 'remove_stale_rule': return 'Stale responsibility 규칙 제거';
    default: return 'WatchTower 내부 해결 불가';
  }
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

function scopedTrack(item: DashboardTrack, runs: WorkflowRunSummary[]): DashboardTrack {
  const running = runs.filter(run => run.status === 'in_progress');
  const queued = runs.filter(run => ['queued', 'requested', 'pending', 'waiting'].includes(run.status));
  const latest = runs[0];
  const health: DashboardTrack['health'] = running.length > 0
    ? 'running'
    : queued.length > 0
      ? 'queued'
      : !latest
        ? 'waiting'
        : latest.status === 'completed'
          ? latest.conclusion === 'success'
            ? 'green'
            : ['failure', 'cancelled', 'timed_out', 'action_required', 'startup_failure', 'stale'].includes(latest.conclusion ?? '')
              ? 'red'
              : 'completed_other'
          : 'waiting';

  return {
    ...item,
    runs,
    health,
    elapsedSeconds: [...running, ...queued].reduce(
      (max, run) => Math.max(max, run.elapsedSeconds),
      0,
    ),
  };
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
  const [dynamicRuleName, setDynamicRuleName] = useState('');
  const [dynamicRuleRepositoryId, setDynamicRuleRepositoryId] = useState<ScopeFilter>('all');
  const [token, setToken] = useState('');
  const [editingId, setEditingId] = useState<number | null>(null);
  const [selectedProject, setSelectedProject] = useState<ScopeFilter>('all');
  const [selectedRepository, setSelectedRepository] = useState<ScopeFilter>('all');
  const [selectedView, setSelectedView] = useState<ViewFilter>('all');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [auditRun, setAuditRun] = useState<WorkflowRunSummary | null>(null);
  const [auditDetail, setAuditDetail] = useState<RunAttributionDetail | null>(null);
  const [auditLoadingId, setAuditLoadingId] = useState<number | null>(null);
  const [resolutionPreview, setResolutionPreview] = useState<ResponsibilityResolutionPreview | null>(null);
  const [resolutionLoadingKey, setResolutionLoadingKey] = useState<string | null>(null);

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
    return base.map(item => scopedTrack(item, item.runs.filter(runInScope)));
  }, [runInScope, selectedView, tracksInProject]);

  const visibleProjectRuns = useMemo(
    () => (dashboard?.projectRuns ?? []).filter(runInScope),
    [dashboard, runInScope],
  );

  const visibleUnassignedRuns = useMemo(
    () => (dashboard?.unassignedRuns ?? []).filter(runInScope),
    [dashboard, runInScope],
  );

  const scopeStats = useMemo(
    () => (dashboard?.repositoryScopeStats ?? []).filter(item =>
      (selectedProject === 'all' || item.projectId === selectedProject) &&
      (selectedRepository === 'all' || item.repositoryId === selectedRepository)
    ),
    [dashboard, selectedProject, selectedRepository],
  );
  const scopedUnassignedCount = scopeStats.reduce((sum, item) => sum + item.unassignedCount, 0);
  const scopedProjectRunCount = scopeStats.reduce((sum, item) => sum + item.projectRunCount, 0);

  const producerStatsInScope = useMemo(
    () => (dashboard?.producerContractStats ?? []).filter(item =>
      (selectedProject === 'all' || item.projectId === selectedProject) &&
      (selectedRepository === 'all' || item.repositoryId === selectedRepository)
    ),
    [dashboard, selectedProject, selectedRepository],
  );
  const producerContract = producerStatsInScope.reduce(
    (total, item) => ({
      sampledRuns: total.sampledRuns + item.sampledRuns,
      projectWideRuns: total.projectWideRuns + item.projectWideRuns,
      explicitRuns: total.explicitRuns + item.explicitRuns,
      runNameRuns: total.runNameRuns + item.runNameRuns,
      prMarkerRuns: total.prMarkerRuns + item.prMarkerRuns,
      commitMarkerRuns: total.commitMarkerRuns + item.commitMarkerRuns,
      branchRuns: total.branchRuns + item.branchRuns,
      heuristicRuns: total.heuristicRuns + item.heuristicRuns,
      manualRuns: total.manualRuns + item.manualRuns,
      compatibilityRuns: total.compatibilityRuns + item.compatibilityRuns,
      unresolvedRuns: total.unresolvedRuns + item.unresolvedRuns,
      otherRuns: total.otherRuns + item.otherRuns,
    }),
    {
      sampledRuns: 0,
      projectWideRuns: 0,
      explicitRuns: 0,
      runNameRuns: 0,
      prMarkerRuns: 0,
      commitMarkerRuns: 0,
      branchRuns: 0,
      heuristicRuns: 0,
      manualRuns: 0,
      compatibilityRuns: 0,
      unresolvedRuns: 0,
      otherRuns: 0,
    },
  );
  const producerContractRunsInScope = useMemo(
    () => (dashboard?.producerContractRuns ?? []).filter(item => runInScope(item.run)),
    [dashboard, runInScope],
  );
  const currentProducerRuns = useMemo(
    () => producerContractRunsInScope.filter(item => item.isCurrentProducerRun),
    [producerContractRunsInScope],
  );
  const currentProducerDriftRuns = useMemo(
    () => currentProducerRuns.filter(item => !item.contractCompliant),
    [currentProducerRuns],
  );
  const historicalProducerDriftRuns = useMemo(
    () => producerContractRunsInScope.filter(item => !item.contractCompliant && !item.isCurrentProducerRun),
    [producerContractRunsInScope],
  );
  const responsibilityReviewRuns = useMemo(
    () => currentProducerRuns.filter(item => item.contractCompliant && !item.responsibilityDeclared),
    [currentProducerRuns],
  );
  const responsibilityMapDriftsInScope = useMemo(
    () => (dashboard?.responsibilityMapDrifts ?? []).filter(item =>
      (selectedProject === 'all' || item.projectId === selectedProject) &&
      (selectedRepository === 'all' || item.repositoryId === selectedRepository)
    ),
    [dashboard, selectedProject, selectedRepository],
  );
  const responsibilityMapSourcesInScope = useMemo(
    () => (dashboard?.responsibilityMapSources ?? []).filter(item =>
      (selectedProject === 'all' || item.projectId === selectedProject) &&
      (selectedRepository === 'all' || item.repositoryId === selectedRepository)
    ),
    [dashboard, selectedProject, selectedRepository],
  );
  const responsibilityMapSourceWarnings = responsibilityMapSourcesInScope.filter(item =>
    item.status === 'error' || (item.status === 'not_found' && item.contractCount > 0)
  );
  const responsibilityMapSourcePending = responsibilityMapSourcesInScope.filter(item => item.status === 'pending');
  const responsibilityMapNotConfigured = responsibilityMapSourcesInScope.filter(
    item => item.status === 'not_found' && item.contractCount === 0,
  );
  const responsibilityMapSynced = responsibilityMapSourcesInScope.filter(item => item.status === 'synced');
  const contractCompliantRuns = currentProducerRuns.filter(item => item.contractCompliant).length;
  const contractDriftRuns = currentProducerDriftRuns.length;
  const currentUnresolvedDriftRuns = currentProducerDriftRuns.filter(
    item => item.bucket === 'unassigned' || item.bucket === 'conflict',
  ).length;
  const contractCoverage = currentProducerRuns.length === 0
    ? 100
    : Math.round((contractCompliantRuns / currentProducerRuns.length) * 100);
  const auditProducerContext = useMemo(
    () => auditRun
      ? (dashboard?.producerContractRuns ?? []).find(item => item.run.id === auditRun.id) ?? null
      : null,
    [auditRun, dashboard],
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

  const openResolutionPreview = async (item: ResponsibilityMapDrift) => {
    try {
      setError(null);
      setNotice(null);
      setResolutionLoadingKey(item.reviewKey);
      setResolutionPreview(await api.getResponsibilityResolutionPreview({ reviewKey: item.reviewKey }));
    } catch (e) {
      setError(String(e));
    } finally {
      setResolutionLoadingKey(null);
    }
  };

  const deferResolution = async (item: ResponsibilityMapDrift) => {
    try {
      setError(null);
      setNotice(null);
      setResolutionLoadingKey(item.reviewKey);
      const result = await api.deferResponsibilityDrift({
        reviewKey: item.reviewKey,
        fingerprint: item.fingerprint,
      });
      setResolutionPreview(null);
      setNotice(result.status === 'deferred'
        ? 'Responsibility 검토를 보류했습니다. Drift는 숨기지 않고 Deferred 상태로 유지합니다.'
        : 'Responsibility 상태가 변경되어 보류 요청을 적용하지 않았습니다.');
      await refresh(false);
    } catch (e) {
      setError(String(e));
    } finally {
      setResolutionLoadingKey(null);
    }
  };

  const applyResolution = async (preview: ResponsibilityResolutionPreview) => {
    try {
      setError(null);
      setNotice(null);
      setResolutionLoadingKey(preview.reviewKey);
      const result = await api.resolveResponsibilityDrift({
        reviewKey: preview.reviewKey,
        fingerprint: preview.fingerprint,
        action: preview.action,
      });
      setResolutionPreview(null);
      setNotice(
        result.status === 'resolved'
          ? '승인한 Responsibility 변경을 적용했고 Drift 재검증까지 통과했습니다.'
          : result.status === 'stale_rejected'
            ? 'Preview 이후 Responsibility 상태가 변경되어 적용을 중단했습니다. 최신 상태를 다시 검토하십시오.'
            : result.status === 'still_open'
              ? '변경은 적용됐지만 Drift가 남아 있습니다. 최신 Inbox 항목을 다시 검토하십시오.'
              : '현재 상태에서는 해당 Responsibility 변경을 실행할 수 없습니다.',
      );
      await refresh(false);
    } catch (e) {
      setError(String(e));
    } finally {
      setResolutionLoadingKey(null);
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

  const submitDynamicRule = async (e: FormEvent) => {
    e.preventDefault();
    if (!dynamicRuleName.trim() || !selectedProjectId) return;
    await act(async () => {
      await api.saveDynamicWorkflowRule({
        projectId: selectedProjectId,
        repositoryId: dynamicRuleRepositoryId === 'all' ? null : dynamicRuleRepositoryId,
        workflowName: dynamicRuleName.trim(),
      });
      setDynamicRuleName('');
      setDynamicRuleRepositoryId('all');
      setSelectedProject(selectedProjectId);
    }, 'Dynamic Workflow 규칙을 추가했습니다.', true);
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

  const inspectAttribution = async (run: WorkflowRunSummary) => {
    if (auditRun?.id === run.id) {
      setAuditRun(null);
      setAuditDetail(null);
      return;
    }
    try {
      setError(null);
      setAuditRun(run);
      setAuditDetail(null);
      setAuditLoadingId(run.id);
      setAuditDetail(await api.getRunAttribution(run.id));
    } catch (e) {
      setAuditRun(null);
      setAuditDetail(null);
      setError(String(e));
    } finally {
      setAuditLoadingId(null);
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
          <span>Unassigned</span><strong>{scopedUnassignedCount}</strong>
        </button>
        <button className="summary-card summary-button" onClick={() => setSelectedView('project')}>
          <span>Project CI</span><strong>{scopedProjectRunCount}</strong>
        </button>
        <div className={`summary-card congestion ${congestionText.toLowerCase()}`}><span>Queue</span><strong>{congestionText}</strong></div>
      </section>

      <section className="panel producer-contract-panel">
        <div className="producer-contract-head">
          <div>
            <p className="eyebrow">PRODUCER CONTRACT</p>
            <h2>최근 Run 귀속 계약</h2>
            <p className="muted-copy">Repository별 최근 최대 50개 Run을 보존하되, 동일 GitHub Workflow의 최신 Run만 현재 producer 상태로 판정합니다.</p>
          </div>
          <div className={`contract-coverage ${contractDriftRuns > 0 ? (currentUnresolvedDriftRuns > 0 ? 'risk' : 'drift') : responsibilityReviewRuns.length > 0 ? 'drift' : 'healthy'}`}>
            <strong>{contractCoverage}%</strong>
            <span>{contractCompliantRuns}/{currentProducerRuns.length}</span>
          </div>
        </div>
        <div className="metrics-row producer-contract-metrics">
          <div><span>명시 신호 · 표본</span><b>{producerContract.explicitRuns}</b><small>run-name {producerContract.runNameRuns} · PR {producerContract.prMarkerRuns} · commit {producerContract.commitMarkerRuns} · branch {producerContract.branchRuns}</small></div>
          <div><span>Project-wide · 표본</span><b>{producerContract.projectWideRuns}</b><small>Track marker 없이 공용 규칙으로 정상 분류</small></div>
          <div><span>Heuristic / 호환 · 표본</span><b>{producerContract.heuristicRuns + producerContract.compatibilityRuns}</b><small>inference {producerContract.heuristicRuns} · alias {producerContract.compatibilityRuns}</small></div>
          <div><span>수동 / 기타 · 표본</span><b>{producerContract.manualRuns + producerContract.otherRuns}</b><small>manual {producerContract.manualRuns} · other {producerContract.otherRuns}</small></div>
          <div><span>미해결 · 표본</span><b>{producerContract.unresolvedRuns}</b><small>unassigned / conflict · historical 포함</small></div>
        </div>
        <div className="producer-drift">
          <div className="producer-drift-head">
            <div><b>Current Drift</b><span>현재 producer {currentProducerDriftRuns.length}건</span></div>
            <small>같은 workflow_id의 최신 Run만 현재 이상으로 계산합니다.</small>
          </div>
          {currentProducerDriftRuns.length === 0 ? (
            <div className="producer-drift-empty">현재 producer 기준으로 조치가 필요한 계약 drift가 없습니다.</div>
          ) : (
            <div className="producer-drift-list">
              {currentProducerDriftRuns.map(item => (
                <button
                  className={`producer-drift-row ${auditRun?.id === item.run.id ? 'selected' : ''}`}
                  key={item.run.id}
                  onClick={() => void inspectAttribution(item.run)}
                >
                  <span className={`producer-bucket ${item.bucket}`}>{producerBucketLabel(item.bucket)}</span>
                  <span className="producer-drift-main">
                    <b>{item.run.workflowName}</b>
                    <small>{item.run.displayTitle}</small>
                  </span>
                  <span className="producer-drift-repo">{item.run.repository}</span>
                  <span className="producer-drift-source">{item.run.attributionSource ?? item.run.resolutionStatus}</span>
                  <span className="mono producer-drift-branch">{item.run.headBranch ?? '—'}</span>
                </button>
              ))}
            </div>
          )}
        </div>
        <div className="producer-drift responsibility-review">
          <div className="producer-drift-head">
            <div><b>Responsibility Review</b><span>현재 producer {responsibilityReviewRuns.length}건</span></div>
            <small>Run은 명시 귀속됐지만 Workflow 책임이 Track-owned / Project-wide / Dynamic으로 선언되지 않은 경우입니다.</small>
          </div>
          {responsibilityReviewRuns.length === 0 ? (
            <div className="producer-drift-empty">현재 producer의 Workflow 책임 계약이 모두 선언되어 있습니다.</div>
          ) : (
            <div className="producer-drift-list">
              {responsibilityReviewRuns.map(item => (
                <button
                  className={`producer-drift-row ${auditRun?.id === item.run.id ? 'selected' : ''}`}
                  key={item.run.id}
                  onClick={() => void inspectAttribution(item.run)}
                >
                  <span className={`producer-bucket ${item.bucket}`}>{producerBucketLabel(item.bucket)}</span>
                  <span className="producer-drift-main">
                    <b>{item.run.workflowName}</b>
                    <small>{item.run.displayTitle}</small>
                  </span>
                  <span className="producer-drift-repo">{item.run.repository}</span>
                  <span className="producer-drift-source">{item.run.attributionSource ?? item.run.resolutionStatus}</span>
                  <span className="mono producer-drift-branch">{item.run.headBranch ?? '—'}</span>
                </button>
              ))}
            </div>
          )}
        </div>
        <div className="producer-drift responsibility-map-drift">
          <div className="producer-drift-head">
            <div>
              <b>Responsibility Map Drift</b>
              <span>detect-only {responsibilityMapDriftsInScope.length}건 · source synced {responsibilityMapSynced.length}</span>
            </div>
            <small>Repository-owned responsibility map과 WatchTower 선언만 비교합니다. Track/규칙/Run 귀속은 자동 변경하지 않습니다.</small>
          </div>
          {responsibilityMapSourcesInScope.length > 0 && (
            <div className="responsibility-source-list">
              {responsibilityMapSourcesInScope.map(source => (
                <div
                  className={'responsibility-source-row ' + source.status + (source.status === 'not_found' && source.contractCount > 0 ? ' stale' : '')}
                  key={source.repositoryId}
                >
                  <span className="responsibility-source-state">{source.status.toUpperCase()}</span>
                  <span className="producer-drift-main">
                    <b>{source.repository}</b>
                    <small>{source.sourcePath}</small>
                  </span>
                  <span className="producer-drift-source">
                    {source.contractCount > 0 ? source.contractCount + ' contracts' : 'no snapshot'}
                  </span>
                  <span className="mono producer-drift-branch">
                    {source.lastSuccessAt ? 'last good ' + source.lastSuccessAt : source.lastAttemptAt ?? 'not polled'}
                  </span>
                </div>
              ))}
            </div>
          )}
          {responsibilityMapSourceWarnings.length > 0 ? (
            <div className="producer-drift-source-warning">
              responsibility map source가 현재 신뢰 가능하지 않습니다. 마지막 정상 snapshot은 유지되지만 0건을 clean으로 판정하지 않습니다.
              {responsibilityMapSourceWarnings.map(source => (
                <small key={source.repositoryId}>
                  {source.repository}: {source.lastError ?? (source.status === 'not_found' ? 'map not found after prior snapshot' : source.status)}
                </small>
              ))}
            </div>
          ) : responsibilityMapSourcePending.length > 0 && responsibilityMapSynced.length === 0 ? (
            <div className="producer-drift-source-pending">아직 responsibility map source를 poll하지 않았습니다.</div>
          ) : responsibilityMapNotConfigured.length > 0 && responsibilityMapSynced.length === 0 ? (
            <div className="producer-drift-source-pending">
              이 범위에는 repository responsibility map이 구성되어 있지 않아 map drift 판정을 수행하지 않습니다.
            </div>
          ) : responsibilityMapDriftsInScope.length === 0 ? (
            <div className="producer-drift-empty">현재 동기화된 repository responsibility map과 WatchTower 책임 선언이 일치합니다.</div>
          ) : (
            <div className="responsibility-review-inbox">
              {responsibilityMapDriftsInScope.map(item => (
                <article
                  className="responsibility-review-card"
                  key={item.repositoryId + ':' + item.workflowPath + ':' + item.driftType}
                >
                  <div className="responsibility-review-card-head">
                    <span className="producer-bucket">{responsibilityDriftLabel(item.driftType)}</span>
                    <span className="producer-drift-main">
                      <b>{item.workflowName}</b>
                      <small>{item.repository}</small>
                    </span>
                    <span className="responsibility-review-head-actions">
                      <span className={`responsibility-review-state ${item.reviewStatus}`}>{item.reviewStatus.toUpperCase()}</span>
                      <span className="responsibility-review-action">{responsibilityActionLabel(item.recommendedAction)}</span>
                    </span>
                  </div>
                  <div className="responsibility-review-contract-grid">
                    <div>
                      <span>Repository contract</span>
                      <b>{item.repositoryBinding}</b>
                      <small>{item.sourcePath} · {item.workflowPath}</small>
                    </div>
                    <div>
                      <span>WatchTower declaration</span>
                      <b>{item.watchtowerBinding ?? '미선언'}</b>
                      <small>
                        {item.expectedTrackKey || item.actualTrackKey
                          ? 'expected ' + (item.expectedTrackKey ?? '—') + ' · observed ' + (item.actualTrackKey ?? '—')
                          : 'Track binding 없음'}
                      </small>
                    </div>
                  </div>
                  <div className="responsibility-review-reason">
                    <span>충돌 이유</span>
                    <p>{item.reason}</p>
                  </div>
                  <div className="responsibility-review-foot">
                    <span>{responsibilityActionLabel(item.recommendedAction)}</span>
                    <span className="responsibility-review-buttons">
                      {item.reviewStatus !== 'blocked' && (
                        <button
                          type="button"
                          className="ghost tiny"
                          disabled={resolutionLoadingKey === item.reviewKey}
                          onClick={() => void deferResolution(item)}
                        >
                          보류
                        </button>
                      )}
                      <button
                        type="button"
                        className="primary tiny"
                        disabled={resolutionLoadingKey === item.reviewKey}
                        onClick={() => void openResolutionPreview(item)}
                      >
                        {resolutionLoadingKey === item.reviewKey ? '확인 중…' : '변경 내용 검토'}
                      </button>
                    </span>
                  </div>
                  {resolutionPreview?.reviewKey === item.reviewKey && (
                    <div className="responsibility-resolution-preview">
                      <div className="responsibility-resolution-preview-head">
                        <div>
                          <span>Resolution Preview</span>
                          <b>{resolutionActionLabel(resolutionPreview.action)}</b>
                        </div>
                        <button type="button" className="ghost tiny" onClick={() => setResolutionPreview(null)}>닫기</button>
                      </div>
                      <div className="responsibility-resolution-summary">
                        <div><span>Repository authority</span><b>{resolutionPreview.repositoryContract}</b></div>
                        <div><span>현재 WatchTower</span><b>{resolutionPreview.watchtowerContract ?? '미선언'}</b></div>
                      </div>
                      {resolutionPreview.blockedReason && (
                        <div className="responsibility-resolution-blocked">{resolutionPreview.blockedReason}</div>
                      )}
                      {resolutionPreview.changes.length > 0 && (
                        <div className="responsibility-resolution-list">
                          <span>적용될 변경</span>
                          {resolutionPreview.changes.map(change => <code key={change}>{change}</code>)}
                        </div>
                      )}
                      <div className="responsibility-resolution-list">
                        <span>변경되지 않는 것</span>
                        {resolutionPreview.invariants.map(invariant => <small key={invariant}>✓ {invariant}</small>)}
                      </div>
                      <div className="responsibility-resolution-confirm">
                        <small>
                          실행 직전 fingerprint를 다시 검증합니다. Preview 이후 상태가 바뀌었으면 변경을 거부합니다.
                        </small>
                        {resolutionPreview.executable && (
                          <button
                            type="button"
                            className="primary"
                            disabled={resolutionLoadingKey === item.reviewKey}
                            onClick={() => void applyResolution(resolutionPreview)}
                          >
                            승인 및 적용
                          </button>
                        )}
                      </div>
                    </div>
                  )}
                </article>
              ))}
            </div>
          )}
        </div>
        {historicalProducerDriftRuns.length > 0 && (
          <div className="producer-drift historical-drift">
            <div className="producer-drift-head">
              <div><b>Historical Drift</b><span>최근 표본 {historicalProducerDriftRuns.length}건</span></div>
              <small>더 최신 Run이 존재하는 과거 evidence입니다. 현재 producer 이상에는 포함하지 않습니다.</small>
            </div>
            <div className="producer-drift-list">
              {historicalProducerDriftRuns.map(item => (
                <button
                  className={`producer-drift-row historical ${auditRun?.id === item.run.id ? 'selected' : ''}`}
                  key={item.run.id}
                  onClick={() => void inspectAttribution(item.run)}
                >
                  <span className={`producer-bucket ${item.bucket}`}>{producerBucketLabel(item.bucket)}</span>
                  <span className="producer-drift-main">
                    <b>{item.run.workflowName}</b>
                    <small>{item.run.displayTitle}</small>
                  </span>
                  <span className="producer-drift-repo">{item.run.repository}</span>
                  <span className="producer-drift-source">{item.run.attributionSource ?? item.run.resolutionStatus}</span>
                  <span className="mono producer-drift-branch">{item.run.headBranch ?? '—'}</span>
                </button>
              ))}
            </div>
          </div>
        )}
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
          <h2>Dynamic Workflow 규칙</h2>
          <p className="hint">여러 Track이 같은 Workflow를 의도적으로 공유할 때 등록합니다. 이 규칙은 Track을 강제하지 않으며 각 Run의 명시 신호는 계속 필요합니다.</p>
          <form onSubmit={submitDynamicRule} className="stack-form">
            <label>Workflow 이름<input value={dynamicRuleName} onChange={e => setDynamicRuleName(e.target.value)} placeholder="예: MESH6J Manual Browser Capture Surface CI" /></label>
            <label>저장소 범위
              <select value={dynamicRuleRepositoryId} onChange={e => setDynamicRuleRepositoryId(e.target.value === 'all' ? 'all' : Number(e.target.value))}>
                <option value="all">프로젝트 전체</option>
                {selectedProjectRepos.map(repo => <option key={repo.id} value={repo.id}>{repo.repo}</option>)}
              </select>
            </label>
            <button className="primary" type="submit" disabled={!selectedProjectId}>Dynamic 규칙 추가</button>
          </form>
          <div className="rule-list">
            {dashboard?.dynamicWorkflowRules.filter(rule => selectedProject === 'all' || rule.projectId === selectedProject).map(rule => (
              <div className="rule-item" key={rule.id}>
                <div>
                  <b>{rule.workflowName}</b>
                  <span>{rule.repositoryId ? dashboard.repositories.find(repo => repo.id === rule.repositoryId)?.repo : '프로젝트 전체'}{rule.protected ? ' · 기본 계약' : ''}</span>
                </div>
                <button
                  className="danger-ghost tiny"
                  disabled={rule.protected}
                  title={rule.protected ? '기본 Dynamic Workflow 계약은 삭제할 수 없습니다.' : undefined}
                  onClick={() => void act(() => api.deleteDynamicWorkflowRule(rule.id), 'Dynamic Workflow 규칙을 삭제했습니다.', true)}
                >
                  {rule.protected ? '고정' : '삭제'}
                </button>
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
              <button className={`filter-chip project-wide ${selectedView === 'project' ? 'active' : ''}`} onClick={() => setSelectedView('project')}>공용 CI <span>{scopedProjectRunCount}</span></button>
              {tracksInProject.map(item => {
                const activity = trackActivity(scopedTrack(item, item.runs.filter(runInScope)));
                const activeCount = activity.running + activity.queued;
                return (
                  <button key={item.track.id} className={`filter-chip ${selectedView === item.track.id ? 'active' : ''}`} onClick={() => setSelectedView(item.track.id)}>
                    {item.track.name}
                    {activeCount > 0 && <span className="chip-live">{activeCount}</span>}
                    {activeCount === 0 && activity.red > 0 && <span className="chip-red">RED</span>}
                  </button>
                );
              })}
              <button className={`filter-chip unassigned ${selectedView === 'unassigned' ? 'active' : ''}`} onClick={() => setSelectedView('unassigned')}>미귀속 <span>{scopedUnassignedCount}</span></button>
            </div>
          </section>

          {auditRun && (
            <section className="panel attribution-audit-panel">
              <div className="attribution-audit-head">
                <div>
                  <p className="eyebrow">ATTRIBUTION AUDIT</p>
                  <h2>{auditRun.workflowName}</h2>
                  <p className="muted-copy">{auditRun.repository} · Run #{auditRun.id} · <span className="mono">{auditRun.headSha.slice(0, 8)}</span></p>
                  {auditProducerContext && (
                    <span className={`audit-producer-context ${auditProducerContext.isCurrentProducerRun ? 'current' : 'historical'}`}>
                      {auditProducerContext.isCurrentProducerRun ? 'CURRENT PRODUCER' : 'HISTORICAL EVIDENCE'} · {producerBucketLabel(auditProducerContext.bucket)}
                    </span>
                  )}
                </div>
                <div className="row-actions">
                  <button className="ghost small" onClick={() => void api.openExternal(auditRun.htmlUrl)}>Actions</button>
                  <button className="ghost small" onClick={() => { setAuditRun(null); setAuditDetail(null); }}>닫기</button>
                </div>
              </div>
              {auditLoadingId === auditRun.id || !auditDetail ? (
                <p className="muted-copy">귀속 근거를 불러오는 중입니다.</p>
              ) : (
                <>
                  <div className="audit-summary-grid">
                    <div><span>최종 상태</span><b>{auditDetail.resolutionStatus}</b></div>
                    <div><span>최종 귀속</span><b>{auditDetail.assignedTrackName ? `${auditDetail.assignedTrackName} / ${auditDetail.assignedTrackKey}` : auditDetail.resolutionStatus === 'project' ? 'Project-wide CI' : '미귀속'}</b></div>
                    <div><span>판정 소스</span><b>{auditDetail.source ?? '—'}</b></div>
                    <div><span>신뢰도</span><b>{auditDetail.confidence ?? 0}</b></div>
                    <div><span>수동 보호</span><b>{auditDetail.manual === true ? 'YES' : auditDetail.manual === false ? 'NO' : '—'}</b></div>
                  </div>
                  <div className="audit-reason">
                    <span>최종 판정 이유</span>
                    <p>{auditDetail.reason ?? '확정 판정 이유가 기록되지 않았습니다.'}</p>
                    {auditDetail.projectRuleId != null && <small>Project rule #{auditDetail.projectRuleId} · {auditDetail.projectRuleRepositoryId == null ? '프로젝트 전체 범위' : 'Repository 전용 범위'}</small>}
                  </div>
                  <div className="audit-evidence-list">
                    <div className="audit-evidence-head"><b>관측 근거</b><span>{auditDetail.evidence.length}건</span></div>
                    {auditDetail.evidence.length === 0 ? (
                      <p className="muted-copy">저장된 resolver evidence가 없습니다. Project-wide 규칙 또는 수동 판정 자체가 최종 근거일 수 있습니다.</p>
                    ) : auditDetail.evidence.map((item, index) => (
                      <div className="audit-evidence-row" key={`${item.signalType}-${item.trackKey}-${index}`}>
                        <span className="audit-score">{item.score}</span>
                        <span><b>{item.signalType}</b><small className="mono">{item.trackKey}</small></span>
                        <code>{item.value}</code>
                      </div>
                    ))}
                  </div>
                  <div className="reconciliation-history">
                    <div className="audit-evidence-head"><b>Assignment Reconciliation History</b><span>{auditDetail.reconciliationHistory.length}건</span></div>
                    {auditDetail.reconciliationHistory.length === 0 ? (
                      <p className="muted-copy">과거 미귀속/충돌 Run의 최종 판정이 변경된 이력이 없습니다.</p>
                    ) : auditDetail.reconciliationHistory.map(entry => (
                      <div className="reconciliation-entry" key={entry.id}>
                        <div className="reconciliation-transition">
                          <span className="reconciliation-time">{new Date(entry.reconciledAt).toLocaleString()}</span>
                          <b>{entry.fromStatus}{entry.fromTrackKey ? ` / ${entry.fromTrackKey}` : ''}</b>
                          <span>→</span>
                          <b>{entry.toStatus}{entry.toTrackKey ? ` / ${entry.toTrackKey}` : ''}</b>
                          <span className="reconciliation-trigger">{entry.trigger}</span>
                        </div>
                        <p>{entry.toReason ?? '판정 이유 없음'}{entry.toSource ? ` · ${entry.toSource}` : ''}{entry.toConfidence != null ? ` · confidence ${entry.toConfidence}` : ''}</p>
                        {entry.evidence.length > 0 && (
                          <div className="reconciliation-evidence">
                            {entry.evidence.map((item, index) => (
                              <span key={`${entry.id}-${item.signalType}-${index}`}><b>{item.score}</b> {item.signalType} · <span className="mono">{item.trackKey}</span></span>
                            ))}
                          </div>
                        )}
                      </div>
                    ))}
                  </div>
                </>
              )}
            </section>
          )}

          {selectedView === 'project' && (
            <section className="panel inbox-panel">
              <div className="section-head">
                <div>
                  <p className="eyebrow">PROJECT-WIDE CI</p>
                  <h2>공용 CI</h2>
                  <p className="muted-copy">표시 {visibleProjectRuns.length}건 · 현재 범위 전체 {scopedProjectRunCount}건 · 저장소별 최대 최근 200건 표시</p>
                </div>
                <strong>{scopedProjectRunCount}</strong>
              </div>
              {scopedProjectRunCount === 0 ? <p className="muted-copy">현재 범위에 공용 CI가 없습니다.</p> : visibleProjectRuns.length === 0 ? <p className="muted-copy">현재 범위에 공용 CI가 있지만 표시 한도를 벗어났습니다.</p> : visibleProjectRuns.map(run => (
                <div className={`run-row project-run-row ${auditRun?.id === run.id ? 'audit-selected' : ''}`} key={run.id}>
                  <span className={`dot ${runDot(run)}`} />
                  <button className="run-main run-link" onClick={() => void api.openExternal(run.htmlUrl)}><b>{run.workflowName}</b><small>{projectById.get(run.projectId)?.name} · {run.repository} · {run.headBranch ?? 'detached'} · <span className="mono">{run.headSha.slice(0, 8)}</span></small></button>
                  <span className="run-state">{run.status}{run.conclusion ? ` · ${run.conclusion}` : ''}</span>
                  <span>{formatDuration(run.elapsedSeconds)}</span>
                  <button className="audit-trigger" onClick={() => void inspectAttribution(run)}>근거 · project</button>
                </div>
              ))}
            </section>
          )}

          {selectedView === 'unassigned' && (
            <section className="panel inbox-panel">
              <div className="section-head">
                <div><p className="eyebrow">ATTRIBUTION INBOX</p><h2>미귀속 CI</h2><p className="muted-copy">표시 {visibleUnassignedRuns.length}건 · 현재 범위 전체 {scopedUnassignedCount}건 · 저장소별 최대 최근 200건 표시</p></div>
              </div>
              {scopedUnassignedCount === 0 ? <p className="muted-copy">현재 확인이 필요한 미귀속 CI가 없습니다.</p> : visibleUnassignedRuns.length === 0 ? <p className="muted-copy">현재 범위에 미귀속 CI가 있지만 표시 한도를 벗어났습니다.</p> : visibleUnassignedRuns.map(run => (
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
                    <button className="ghost small audit-action" onClick={() => void inspectAttribution(run)}>{auditRun?.id === run.id ? '근거 닫기' : '귀속 근거'}</button>
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
                  <div><span>트랙 전체 최근 20회 평균</span><b>{formatDuration(item.averageDurationSeconds)}</b></div>
                  <div><span>장기 기준</span><b>{item.track.longCiMinutes}m</b></div>
                </div>
                <div className="runs-list">
                  {item.runs.length === 0 ? <p className="muted-copy">현재 Repository 필터에 표시할 Run이 없습니다.</p> : item.runs.map(run => (
                    <div className={`run-row v2 ${auditRun?.id === run.id ? 'audit-selected' : ''}`} key={run.id}>
                      <span className={`dot ${runDot(run)}`} />
                      <button className="run-main run-link" onClick={() => void api.openExternal(run.htmlUrl)}><b>{run.workflowName}</b><small>{run.repository} · {run.headBranch ?? 'detached'} · <span className="mono">{run.headSha.slice(0, 8)}</span></small></button>
                      <span className="run-state">{run.status}{run.conclusion ? ` · ${run.conclusion}` : ''}</span>
                      <span>{formatDuration(run.elapsedSeconds)}</span>
                      <button className="attribution audit-trigger" onClick={() => void inspectAttribution(run)}>근거 · {run.attributionSource ?? 'resolver'} · {run.confidence ?? 0}</button>
                    </div>
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
