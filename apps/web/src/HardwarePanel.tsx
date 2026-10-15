import {useState} from 'react';
import type {Rover} from './generated';
import styles from './App.module.css';
import {readinessProblems} from './geometry';
type Props={rover:Rover;canCommand:boolean;pending?:boolean;onRequest:(action:'arm'|'disarm',run:string,revision:string)=>Promise<void>};
export function HardwarePanel({rover,canCommand,pending=false,onRequest}:Props){
 const [confirmation,setConfirmation]=useState(''),[busy,setBusy]=useState(false),[error,setError]=useState('');
 const a=rover.authority,h=rover.hardware;
 const recent=(age:number|null|undefined)=>age!==null&&age!==undefined&&Number.isFinite(age)&&age>=0&&age<0.5;
 const canArm=canCommand&&!pending&&rover.membership==='online'&&!!rover.pose&&recent(rover.pose_age)&&!!h?.ready&&!h.armed&&recent(h.age)&&!!a?.run_id&&a.safety==='clear'&&recent(a.age);
 const key=JSON.stringify([rover.id,a?.run_id,a?.revision,h?.armed,canCommand]);
 async function request(action:'arm'|'disarm'){
  if(busy||!canCommand||(action==='arm'&&(!canArm||confirmation!==key)))return;
  setBusy(true);setError('');setConfirmation('');
  try{await onRequest(action,a?.run_id??'',a?.revision??'')}catch(e){setError(e instanceof Error?e.message:'Hardware outcome uncertain; check live status.')}finally{setBusy(false)}
 }
 return <section className={styles.hardwareSection} aria-label="Hardware controls">
  <h3 className="yg-title">Vehicle power</h3>
  <div className={styles.buttons}><button disabled={!canArm||busy} onClick={()=>{setConfirmation(key);setError('')}}>Arm vehicle</button><button className={styles.cancel} disabled={!canCommand||busy} onClick={()=>void request('disarm')}>Disarm vehicle</button></div>
  {confirmation===key&&<div className={styles.confirmation} role="region" aria-label="Confirm arming"><p>Enable motors on rover {rover.id}. Make sure the vehicle is ready to move.</p><button className={styles.primary} disabled={!canArm||busy} onClick={()=>void request('arm')}>Confirm arm rover {rover.id}</button><button onClick={()=>setConfirmation('')}>Back</button></div>}
  {!h?.armed&&!canArm&&<p className={styles.support}>{!canCommand?'Take control to arm this vehicle.':pending?'A command is awaiting confirmation.':readinessProblems(rover,true).join('; ')}</p>}
  {error&&<p role="alert" className={styles.notice}>{error}</p>}
 </section>
}
