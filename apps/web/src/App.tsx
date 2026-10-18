import { lazy, Suspense, useCallback, useEffect, useState } from 'react';
import { api, ApiError, clientID } from './api';
import type { Identity, NavigationParameters, Operation, PublicConfig, Waypoint } from './generated';
import { useFleet } from './useFleet';
import {useOccupancy} from './useOccupancy';
import { ready } from './geometry';
import { LocalMap } from './LocalMap';
import { WaypointPanel } from './WaypointPanel';
import styles from './App.module.css';
import {DrivePanel} from './DrivePanel';
const GeographicMap = lazy(() => import('./GeographicMap'));
export default function App() {
  const [user, setUser] = useState<Identity | null>(null), [config, setConfig] = useState<PublicConfig | null>(null), [loading, setLoading] = useState(true), [error, setError] = useState('');
  const [draftSource, setDraftSource] = useState<{ id: string; run_id: string; frame_id: string | null } | null>(null);
  const [selection, setSelection] = useState(''), [frame, setFrame] = useState<'local' | 'geographic'>('local'), [draft, setDraft] = useState<Waypoint | null>(null), [busy, setBusy] = useState(false);
  const clearDraft = useCallback(() => { setDraft(null); setDraftSource(null) }, []);
  const { fleet, connected } = useFleet(user, clearDraft);
  useEffect(() => { let alive = true; void api<Identity>('/me').then(async u => { const c = await api<PublicConfig>('/config'); if (alive) { setUser(u); setConfig(c) } }).catch(e => { if (alive && (!(e instanceof ApiError) || e.status !== 401)) setError(e.message) }).finally(() => { if (alive) setLoading(false) }); return () => { alive = false } }, []);
  const selected = fleet?.rovers.find(r => r.id === selection);
  const {map:occupancy,status:occupancyStatus}=useOccupancy(user,selected,connected);
  const operation = fleet?.operations.find(o => o.rover_id === selected?.id);
  const hasControl = !!fleet?.claim?.owner.endsWith(`:${clientID}`) && fleet.claim.expires_at > fleet.now;
  const canCommand = user?.role === 'operator' && hasControl && connected;
  useEffect(() => { setDraft(null) }, [selected?.id, selected?.authority?.run_id, selected?.frame_id, frame]);
  useEffect(() => { if (!hasControl || !user || !connected) return; const timer = setInterval(() => { void api('/control', user, 'POST').catch(e => setError(e.message)) }, 10000); return () => clearInterval(timer) }, [hasControl, user, connected]);
  async function control() { if (!user) return; setBusy(true); setError(''); try { await api('/control', user, hasControl ? 'DELETE' : 'POST') } catch (e) { setError((e as Error).message) } finally { setBusy(false) } }
  function chooseDraft(w: Waypoint | null) { setDraft(w); setDraftSource(w && selected ? { id: selected.id, run_id: selected.authority?.run_id ?? '', frame_id: selected.frame_id } : null) }
  async function send(w: Waypoint, request: string) { if (!selected || !user || !draftSource) throw new Error('Review a new waypoint'); if (draftSource.id !== selected.id) throw new Error('Rover selection changed'); await api<Operation>(`/rovers/${selected.id}/waypoints`, user, 'POST', { request_id: request, waypoint: w, run_id: draftSource.run_id, frame_id: draftSource.frame_id }) }
  async function cancel(request: string) { if (!selected || !user) throw new Error('Select a rover'); await api<Operation>(`/rovers/${selected.id}/cancel`, user, 'POST', { request_id: request }) }
  async function navigation(parameters:NavigationParameters,run:string,revision:string,settingsRevision:string){if(!selected||!user)throw new Error('Select a rover');await api<Operation>(`/rovers/${selected.id}/navigation`,user,'POST',{request_id:crypto.randomUUID(),parameters,run_id:run,revision,settings_revision:settingsRevision})}
  async function autonomy(level:string,run:string,revision:string){if(!selected||!user)throw new Error('Select a rover');clearDraft();await api<Operation>(`/rovers/${selected.id}/autonomy`,user,'POST',{request_id:crypto.randomUUID(),level,run_id:run,revision})}
  async function hardware(action: 'arm' | 'disarm', run: string, revision: string) { if (!selected || !user) throw new Error('Select a rover'); await api<Operation>(`/rovers/${selected.id}/hardware`, user, 'POST', { request_id: crypto.randomUUID(), action, run_id: run, revision }) }
  async function logout() { if (!user) return; try { await api('/logout', user, 'POST'); setUser(null); setDraft(null) } catch (e) { setError((e as Error).message) } }
  return <div className={styles.shell}>
    <a href="#main" className={styles.skip}>Skip to fleet</a>
    <header className={styles.header}><a href="/" className={styles.wordmark}>argos<span>Fleet supervision</span></a><div className={styles.account}>{user ? <><span>{user.login} / {user.role}</span><button onClick={() => void logout()}>Sign out</button></> : <span>Adaptive robotic group operator system</span>}</div></header>
    <main id="main">
      {!user ? <section className={styles.login}><p className="yg-label">Operator access / 01</p><h1 className="yg-headline">A view of<br />the whole fleet.</h1><p className="yg-lede">Reported positions. Explicit goals.<br />One shared operating picture.</p>{loading ? <p role="status">Connecting…</p> : <div className={styles.buttons}><a className={styles.primary} href="/auth/github">Sign in with GitHub</a>{import.meta.env.DEV && <a className={styles.secondary} href="/auth/dev">Open local demo</a>}</div>}<p className={styles.support}>Access is limited to invited team members.</p>{error && <p role="alert">{error}</p>}</section> : <>
        <section className={styles.fleetToolbar} aria-label="Fleet overview"><div><h1 className="yg-title">Fleet field</h1><span className={styles.support}>{fleet?.rovers.length ?? 0} vehicles · {fleet?.rovers.filter(ready).length ?? 0} waypoint ready · {config?.demo ? 'Simulated fleet' : 'Live fleet'}</span></div><div className={styles.connection}><strong>{connected ? 'Stream connected' : 'Stream disconnected'}</strong><span>{connected ? `Fleet link: ${fleet?.link ?? 'waiting'}` : 'Last known data · commands disabled'}</span></div><div className={styles.controlSummary}><span>{hasControl ? 'Held by this tab' : fleet?.claim?.login ?? 'Control available'}</span>{user.role === 'operator' && <button disabled={!connected || busy || !!fleet?.claim && !hasControl} onClick={() => void control()}>{hasControl ? 'Release control' : 'Take control'}</button>}</div></section>
        {error && <div className={styles.notice} role="alert">{error}<button onClick={() => setError('')}>Dismiss</button></div>}
        <div className={styles.workspace}>
          <section className={styles.mapSection} aria-label="Fleet map"><div className={styles.mapHeading}><h2 className="yg-title">Field view</h2><div className={styles.buttons}><button aria-pressed={frame === 'local'} onClick={() => setFrame('local')}>Local</button><button aria-pressed={frame === 'geographic'} onClick={() => setFrame('geographic')}>Geographic</button></div></div>{frame === 'local' ? <LocalMap occupancy={occupancy} occupancyStatus={occupancyStatus} rovers={fleet?.rovers ?? []} selected={selected} onSelect={setSelection} draft={draft} onDraft={chooseDraft} /> : <Suspense fallback={<p>Loading geographic map…</p>}><GeographicMap occupancy={occupancy} occupancyStatus={occupancyStatus} onSelect={setSelection} rovers={fleet?.rovers ?? []} selected={selected} draft={draft} styleURL={config?.tile_style ?? null} onDraft={chooseDraft} onLocal={() => setFrame('local')} /></Suspense>}<p className={styles.figureCaption}>FIG. 01 / Callouts connect to reported positions. Dashed lines mark destinations, not planned paths.</p></section>
          {selected ? <div className={styles.inspectorOverlay}><button className={styles.closeInspector} onClick={() => { setSelection(''); clearDraft() }} aria-label="Close rover controls">Close ×</button><DrivePanel key={selected.id} rover={selected} canCommand={canCommand} linkHealthy={fleet?.link === "healthy"} now={fleet?.now ?? 0} onDrive={async input=>{await api(`/rovers/${selected.id}/drive`,user,'POST',input)}} onTakeover={async()=>{clearDraft();if(!['teleop','assisted_teleop'].includes(selected.authority?.requested_level ?? '')){const op=await api<Operation>(`/rovers/${selected.id}/autonomy`,user,'POST',{request_id:crypto.randomUUID(),level:'teleop',run_id:selected.authority?.run_id ?? '',revision:selected.authority?.revision ?? ''});if(!op.mode_token||op.phase!=='autonomy_pending')throw new Error(op.message);return op.mode_token}}} onStop={async()=>{clearDraft();await api(`/rovers/${selected.id}/stop`,user,'POST')}} /><WaypointPanel rover={selected} frame={frame} draft={draft} onDraft={chooseDraft} canCommand={canCommand} connected={connected} operation={operation} onSend={send} onCancel={cancel} onHardware={hardware} onAutonomy={autonomy} onNavigation={navigation} /></div> : <p className={styles.sceneHint}>Select a rover on the field to inspect it or set a waypoint.</p>}
        </div>

      </>}
    </main><footer className={styles.footer}><span className="yg-label">ARGOS / Operator interface</span><span>Published does not mean accepted.</span></footer>
  </div>
}
