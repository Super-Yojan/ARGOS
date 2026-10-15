import {useEffect,useState} from 'react';
import type {NavigationParameters,Rover} from './generated';
import styles from './App.module.css';
type Props={rover:Rover;canCommand:boolean;pending?:boolean;onRequest:(p:NavigationParameters,run:string,revision:string,settingsRevision:string)=>Promise<void>};
const fields:{key:keyof NavigationParameters;label:string;unit:string;scale:number;step:number;min:number;max:number}[]=[
 {key:'length',label:'Length',unit:'cm',scale:100,step:0.05,min:0.1,max:3},
 {key:'width',label:'Width',unit:'cm',scale:100,step:0.05,min:0.1,max:3},
 {key:'clearance',label:'Clearance',unit:'cm',scale:100,step:0.025,min:0,max:1},
 {key:'max_linear',label:'Planner speed',unit:'m/s',scale:1,step:0.05,min:0.05,max:2},
 {key:'max_angular',label:'Planner turn rate',unit:'rad/s',scale:1,step:0.1,min:0.1,max:2},
 {key:'horizon',label:'Look-ahead',unit:'s',scale:1,step:0.25,min:0.5,max:5},
];
export function NavigationPanel({rover,canCommand,pending=false,onRequest}:Props){
 const n=rover.navigation,a=rover.authority,h=rover.hardware;
 const key=JSON.stringify([rover.id,a?.run_id,n?.revision]);
 const [draft,setDraft]=useState<NavigationParameters|null>(n?.parameters??null),[source,setSource]=useState(key),[busy,setBusy]=useState(false),[error,setError]=useState('');
 useEffect(()=>{setDraft(n?.parameters?{...n.parameters}:null);setSource(key);setError('')},[key]);
 const recent=(age:number|undefined)=>age!==undefined&&Number.isFinite(age)&&age>=0&&age<0.5;
 const available=canCommand&&!pending&&!busy&&source===key&&rover.membership==='online'&&h?.ready&&!h.armed&&h.arming===false&&recent(h.age)&&!!a?.run_id&&recent(a.age)&&['clear','hold'].includes(a.safety)&&n?.schema_version===1&&recent(n.age)&&rover.goal?.state!=='active';
 const valid=!!draft&&fields.every(f=>Number.isFinite(draft[f.key])&&draft[f.key]>=f.min&&draft[f.key]<=f.max);
 const dirty=!!draft&&!!n&&fields.some(f=>draft[f.key]!==n.parameters[f.key]);
 function edit(field:keyof NavigationParameters,value:number){if(draft)setDraft({...draft,[field]:value})}
 async function apply(){if(!available||!valid||!dirty||!draft||!a||!n)return;setBusy(true);setError('');try{await onRequest(draft,a.run_id,a.revision,n.revision)}catch(e){setError(e instanceof Error?e.message:'Settings outcome uncertain; check reported values.')}finally{setBusy(false)}}
 return <details className={styles.navigationSettings}><summary>Dimensions &amp; avoidance</summary>
  {!n||n.schema_version!==1?<p className={styles.support}>Update Terra on the phone to enable remote planner settings.</p>:<>
   <p className={styles.support}>Planning footprint and obstacle-avoidance limits. Session settings are retained until Terra restarts.</p>
   {draft&&<fieldset disabled={!available} className={styles.tuningFields}>{fields.map(f=><div className={styles.tuningRow} key={f.key}>
    <label htmlFor={`tuning-${f.key}`}>{f.label} ({f.unit})</label>
    <div><button type="button" aria-label={`Decrease ${f.label.toLowerCase()}`} disabled={!available||draft[f.key]<=f.min} onClick={()=>edit(f.key,Math.max(f.min,Math.round((draft[f.key]-f.step)*100000)/100000))}>−</button>
    <input id={`tuning-${f.key}`} type="number" inputMode="decimal" min={f.min*f.scale} max={f.max*f.scale} step={f.step*f.scale} value={Number.isFinite(draft[f.key])?Number((draft[f.key]*f.scale).toFixed(3)):''} onChange={e=>edit(f.key,Number(e.target.value)/f.scale)}/>
    <button type="button" aria-label={`Increase ${f.label.toLowerCase()}`} disabled={!available||draft[f.key]>=f.max} onClick={()=>edit(f.key,Math.min(f.max,Math.round((draft[f.key]+f.step)*100000)/100000))}>+</button></div>
   </div>)}</fieldset>}
   {draft&&<p className={styles.support}>Planner radius: {((Math.hypot(draft.length,draft.width)/2+draft.clearance)*100).toFixed(1)} cm. The enclosing circle includes the clearance margin.</p>}
   <div className={styles.buttons}><button disabled={!available||!valid||!dirty} onClick={()=>void apply()}>Apply settings</button><button disabled={!dirty||busy} onClick={()=>{setDraft({...n.parameters});setError('')}}>Reset draft</button></div>
   {!available&&<p className={styles.support}>{!canCommand?'Take control to change settings.':pending?'Wait for the current command or cancel it first.':h?.armed||h?.arming?'Disarm the vehicle before changing settings.':rover.goal?.state==='active'?'Cancel the active goal before changing settings.':'Fresh ready, disarmed hardware and current Terra settings are required.'}</p>}
   {pending&&<p role="status">Waiting for Terra to confirm settings…</p>}
   {error&&<p role="alert" className={styles.notice}>{error}</p>}
  </>}
 </details>
}
