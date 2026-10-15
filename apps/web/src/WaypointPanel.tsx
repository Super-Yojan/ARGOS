import { useEffect,useState } from 'react';
import type { NavigationParameters,Operation,Rover,Waypoint } from './generated';
import { ageLabel,ready,terminal,validWaypoint,readinessProblems,safetyReason } from './geometry';
import styles from './App.module.css';
import {HardwarePanel} from './HardwarePanel';
import {AutonomyPanel} from './AutonomyPanel';
import {NavigationPanel} from './NavigationPanel';
type Props={onNavigation?:(p:NavigationParameters,run:string,revision:string,settingsRevision:string)=>Promise<void>;onAutonomy?:(level:string,run:string,revision:string)=>Promise<void>;onHardware?:(action:'arm'|'disarm',run:string,revision:string)=>Promise<void>;rover:Rover|undefined;frame:'local'|'geographic';draft:Waypoint|null;onDraft:(w:Waypoint|null)=>void;canCommand:boolean;connected:boolean;operation:Operation|undefined;onSend:(w:Waypoint,request:string)=>Promise<void>;onCancel:(request:string)=>Promise<void>};
export function WaypointPanel({rover,frame,draft,onDraft,canCommand,connected,operation,onSend,onCancel,onHardware,onAutonomy,onNavigation}:Props){
  const [first,setFirst]=useState(''),[second,setSecond]=useState(''),[confirmed,setConfirmed]=useState(false),[busy,setBusy]=useState(false),[error,setError]=useState('');
  useEffect(()=>{setConfirmed(false);setError('');if(draft?.frame==='local'){setFirst(String(draft.x));setSecond(String(draft.y))}else if(draft?.frame==='geographic'){setFirst(String(draft.latitude));setSecond(String(draft.longitude))}else{setFirst('');setSecond('')}},[draft,rover?.id,frame]);
  const active=operation&&!terminal(operation.phase);
  const available=!!rover&&ready(rover)&&connected&&(frame==='local'||!!rover.reference);
  function review(){
    const a=Number(first),b=Number(second);const w:Waypoint=frame==='local'?{frame:'local',x:a,y:b,yaw:null}:{frame:'geographic',latitude:a,longitude:b,yaw:null};
    if(!first.trim()||!second.trim()||!validWaypoint(w)){setError(frame==='local'?'Enter finite coordinates within ±20,000 m.':'Latitude must be within ±85° and longitude within ±180°.');return}
    onDraft(w);setTimeout(()=>setConfirmed(true),0);setError('');
  }
  async function send(){if(!draft||busy)return;setBusy(true);setError('');try{await onSend(draft,crypto.randomUUID());setConfirmed(false);onDraft(null)}catch(e){setError(e instanceof Error?e.message:'Request outcome uncertain. Check operation history.');setConfirmed(false)}finally{setBusy(false)}}
  async function cancel(){setBusy(true);setError('');try{await onCancel(crypto.randomUUID());onDraft(null);setConfirmed(false)}catch(e){setError(e instanceof Error?e.message:'Cancellation outcome uncertain.')}finally{setBusy(false)}}
  return <aside className={styles.inspector} aria-label="Rover details">
    <p className="yg-label">02 / Selected vehicle</p>
    <h2 className="yg-title">{rover?`Rover ${rover.id}`:'Select a rover'}</h2>
    {rover&&onHardware&&<HardwarePanel rover={rover} canCommand={canCommand&&connected} pending={!!active} onRequest={onHardware}/>}
    {rover&&onNavigation&&<NavigationPanel rover={rover} canCommand={canCommand&&connected} pending={!!active} onRequest={onNavigation}/>}
    {rover&&onAutonomy&&<AutonomyPanel rover={rover} canCommand={canCommand&&connected} pending={!!active&&operation?.kind.startsWith('autonomy:')} onRequest={onAutonomy}/>}
    {rover&&<><dl className={styles.details}>
      <div><dt>Position</dt><dd>{rover.pose?`${rover.pose.x.toFixed(2)}, ${rover.pose.y.toFixed(2)} m`:'Unknown'}</dd></div>
      <div><dt>Pose received</dt><dd>{ageLabel(rover.pose_age)}</dd></div>
      <div><dt>Heading</dt><dd>{rover.pose?`${(rover.pose.yaw*180/Math.PI).toFixed(1)}°`:'Unknown'}</dd></div>
      <div><dt>Hardware</dt><dd>{!rover.hardware?'Unknown':rover.hardware.armed?'Armed':'Disarmed'}</dd></div>
      <div><dt>Authority</dt><dd>{rover.authority?.requested_level.replaceAll('_',' ')??'Unknown'}</dd></div>
      <div><dt>Safety</dt><dd>{rover.authority?.safety??'Unknown'}{rover.authority?.reason?` · ${safetyReason(rover.authority.reason)}`:''}</dd></div>
      <div><dt>Goal</dt><dd>{rover.goal?.state??'None'}{rover.goal&&rover.goal.state!=='idle'?` · ${rover.goal.distance.toFixed(1)} m`:''}</dd></div>
    </dl><p className={styles.support}>{rover.hardware?.reason??'Hardware status unavailable.'}</p></>}
    <div className={styles.waypointSection}>
      <p className="yg-label">03 / Directed order</p><h2 className="yg-title">Set a waypoint</h2>
      <p className={styles.support}>{frame==='local'?'+x north / forward. +y west / left.':'Latitude and longitude in WGS84.'} Map clicks place a draft.</p>
      <form onSubmit={e=>{e.preventDefault();review()}}>
        <fieldset disabled={!rover||busy} className={styles.fields}>
          <label>{frame==='local'?'North / x (m)':'Latitude (°)'}<input name="first" inputMode="decimal" value={first} onChange={e=>{setFirst(e.target.value);setConfirmed(false)}} placeholder={frame==='local'?'12.0':'38.8298100'} autoComplete="off"/></label>
          <label>{frame==='local'?'West / y (m)':'Longitude (°)'}<input name="second" inputMode="decimal" value={second} onChange={e=>{setSecond(e.target.value);setConfirmed(false)}} placeholder={frame==='local'?'0.0':'−77.3075000'} autoComplete="off"/></label>
          <button type="submit" className={styles.secondary}>Review waypoint</button>
        </fieldset>
      </form>
      {confirmed&&draft&&<div className={styles.confirmation} role="region" aria-label="Confirm waypoint"><strong>Confirm rover {rover?.id}</strong><p>{draft.frame==='local'?`${draft.x} m north, ${draft.y} m west`:`${draft.latitude}°, ${draft.longitude}°`}</p><p>One waypoint will be published. Closing this page will leave a published goal running.</p><button className={styles.primary} disabled={!canCommand||!available||!!active||busy} onClick={()=>void send()}>Send waypoint</button></div>}
      {!available&&rover&&<p className={styles.notice}>{!connected?'Live connection required.':frame==='geographic'&&!rover.reference?'Geographic reference unavailable. Use the local map.':`Waypoint unavailable: ${readinessProblems(rover).join('; ')}.`}</p>}
      {!canCommand&&<p className={styles.support}>An operator must hold control to send or cancel.</p>}
      <button className={styles.cancel} disabled={!canCommand||!connected||!rover||busy} onClick={()=>void cancel()}>Cancel rover goal</button>
      {error&&<p role="alert" className={styles.notice}>{error}</p>}
      {operation&&<div className={styles.operation} role="status"><p className="yg-label">Latest command</p><strong>{operation.phase.replaceAll('_',' ')}</strong><p>{operation.message}</p><small>Request {operation.request_id}</small></div>}
    </div>
  </aside>
}
