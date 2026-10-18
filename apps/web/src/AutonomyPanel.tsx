import {useState} from 'react';
import type {Rover} from './generated';
import {safetyReason} from './geometry';
import styles from './App.module.css';
// Keep the level numbering and labels aligned with apple/Shared/Models/AutonomyChoice.swift.
export const autonomyChoices=[
 {title:'L0 · Fully teleop',wire:'teleop'},
 {title:'L1 · Assisted teleop',wire:'assisted_teleop'},
 {title:'L2 · Waypoint · no avoidance',wire:'waypoint_direct'},
 {title:'L3 · Waypoint · obstacle aware',wire:'waypoint'},
 {title:'L4 · Target search',wire:'target_search'},
 {title:'L5 · Decision making',wire:null},
] as const;
type Props={rover:Rover;canCommand:boolean;pending?:boolean;onRequest:(level:string,run:string,revision:string)=>Promise<void>};
export function AutonomyPanel({rover,canCommand,pending=false,onRequest}:Props){
 const [busy,setBusy]=useState(false),[error,setError]=useState('');const a=rover.authority;
 const recent=(age:number|null|undefined)=>age!=null&&Number.isFinite(age)&&age>=0&&age<0.5;
 const available=canCommand&&!busy&&!pending&&rover.membership==='online'&&!!rover.pose&&recent(rover.pose_age)&&!!a?.run_id&&recent(a.age);
 async function request(level:string){if(!available||!a||!a.supported_levels.includes(level))return;setBusy(true);setError('');try{await onRequest(level,a.run_id,a.revision)}catch(e){setError(e instanceof Error?e.message:'Mode outcome uncertain; check reported authority.')}finally{setBusy(false)}}
 return <section className={styles.hardwareSection} aria-label="Autonomy levels"><h3 className="yg-title">Autonomy level</h3>
  <p className={styles.support}>Reported: {autonomyChoices.find(c=>c.wire===a?.requested_level)?.title??a?.requested_level??'Unknown'}{a?.reason?` · ${safetyReason(a.reason)}`:''}</p>
  <div className={styles.autonomyChoices}>{autonomyChoices.map(c=><button key={c.title} aria-pressed={!!c.wire&&a?.requested_level===c.wire} disabled={!available||!c.wire||!a?.supported_levels.includes(c.wire)} onClick={()=>c.wire&&void request(c.wire)}>{c.title}</button>)}</div>
  <p className={styles.support}>L2 does not avoid obstacles. L3 needs a fresh obstacle map. L4 needs a LiDAR phone with live person detection. L5 is unavailable. Changing mode does not arm the vehicle.</p>
  {pending&&<p role="status">Waiting for vehicle acknowledgement…</p>}
  {!canCommand&&<p className={styles.support}>Take control to change autonomy.</p>}
  {error&&<p role="alert" className={styles.notice}>{error}</p>}
 </section>
}
