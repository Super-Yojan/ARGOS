import {useEffect,useRef,useState} from 'react';
import type {DriveRequest, Rover} from './generated';
import styles from './App.module.css';
let sequence=0;
type Props={rover:Rover;canCommand:boolean;linkHealthy?:boolean;now:number;onDrive:(r:DriveRequest)=>Promise<void>;onTakeover:()=>Promise<string|void>;onStop:()=>Promise<void>};
const keyDirection:Record<string,string>={w:'Forward',ArrowUp:'Forward',s:'Reverse',ArrowDown:'Reverse',a:'Left',ArrowLeft:'Left',d:'Right',ArrowRight:'Right'};
export function DrivePanel(props:Props){
 const [enabled,setEnabled]=useState(false),[busy,setBusy]=useState(false),[error,setError]=useState(''),[pendingToken,setPendingToken]=useState<string|null>(null),[stopStatus,setStopStatus]=useState('');
 const current=useRef(props);current.current=props;
 const held=useRef(new Set<string>()),inflight=useRef(false),active=useRef(false),generation=useRef(0);
 const recent=(age:number|null|undefined)=>age!=null&&Number.isFinite(age)&&age>=0&&age<0.5;
 const a=props.rover.authority,h=props.rover.hardware;
 const ready=props.canCommand&&props.linkHealthy!==false&&props.rover.membership==='online'&&!!props.rover.pose&&recent(props.rover.pose_age)&&!!a?.run_id&&recent(a.age)&&['teleop','assisted_teleop'].includes(a.requested_level)&&['clear','active'].includes(a.safety)&&!!h?.armed&&h.ready&&recent(h.age);
 const available=useRef(false);available.current=enabled&&ready;
 function publish(neutral=false){const p=current.current,a=p.rover.authority;if(!a)return;const keys=held.current;const linear=neutral?0:(Number(keys.has('Forward'))-Number(keys.has('Reverse')))*0.3,angular=neutral?0:(Number(keys.has('Left'))-Number(keys.has('Right')))*0.6;
  if(!neutral&&(inflight.current||!available.current||!keys.size))return;
  if(!neutral)inflight.current=true;
  void p.onDrive({run_id:a.run_id,revision:a.revision,observed_at:p.now,sequence:++sequence,linear,angular}).catch(e=>{held.current.clear();active.current=false;setEnabled(false);setError(e instanceof Error?e.message:'Drive stopped; outcome uncertain')}).finally(()=>{if(!neutral)inflight.current=false});
 }
 function release(){held.current.clear();if(active.current){active.current=false;publish(true)}}
 function disable(){generation.current++;release();setEnabled(false);setPendingToken(null)}
 useEffect(()=>{if(!props.canCommand||props.linkHealthy===false)disable();else if(!ready){release();setEnabled(false)}},[ready,props.canCommand,props.linkHealthy]);
 useEffect(()=>{disable()},[props.rover.id,a?.run_id]);
 useEffect(()=>{release();setEnabled(false)},[a?.revision]);
 useEffect(()=>{if(pendingToken&&a?.token===pendingToken){if(a.result==='accepted'&&ready){setEnabled(true);setPendingToken(null)}else if(a.result&&a.result!=='accepted'){setError('Vehicle rejected manual takeover');setPendingToken(null)}}},[pendingToken,a?.token,a?.result,ready,a?.revision]);
 useEffect(()=>{
  const down=(e:KeyboardEvent)=>{if(e.target instanceof HTMLElement&&(e.target.isContentEditable||['INPUT','TEXTAREA','SELECT'].includes(e.target.tagName)))return;const direction=keyDirection[e.key];if(direction&&available.current){e.preventDefault();held.current.add(direction);active.current=true}};
  const up=(e:KeyboardEvent)=>{const direction=keyDirection[e.key];if(direction){held.current.delete(direction);if(!held.current.size)release()}};
  const blur=()=>disable();const visibility=()=>{if(document.hidden)disable()};
  const timer=setInterval(()=>publish(),200);
  window.addEventListener('keydown',down);window.addEventListener('keyup',up);window.addEventListener('blur',blur);window.addEventListener('pagehide',blur);document.addEventListener('visibilitychange',visibility);
  return()=>{clearInterval(timer);window.removeEventListener('keydown',down);window.removeEventListener('keyup',up);window.removeEventListener('blur',blur);window.removeEventListener('pagehide',blur);document.removeEventListener('visibilitychange',visibility);generation.current++;release()};
 },[]);
 async function takeover(){const epoch=++generation.current;setBusy(true);setError('');try{const token=await props.onTakeover();if(epoch===generation.current){if(token)setPendingToken(token);else setEnabled(true)}}catch(e){setError((e as Error).message)}finally{setBusy(false)}}
 return <section className={styles.hardwareSection} aria-label="Manual driving"><h3 className="yg-title">Manual driving</h3>
 <button disabled={!props.canCommand||busy} onClick={()=>void takeover()}>Take over manual driving</button>
 <p className={styles.support}>{enabled&&ready?'Hold arrow keys, WASD or a direction below. Release to stop.':'Select takeover after manual authority is reported and hardware is armed.'} Maximum 0.3 m/s. Keep the vehicle in view.</p>
 <div className={styles.buttons}>{['Forward','Left','Reverse','Right'].map(direction=><button key={direction} disabled={!enabled||!ready} style={{touchAction:'none'}} onPointerDown={e=>{e.currentTarget.setPointerCapture?.(e.pointerId);held.current.add(direction);active.current=true}} onPointerUp={()=>release()} onPointerCancel={()=>release()} onLostPointerCapture={()=>release()}>{direction}</button>)}</div>
 <button className={styles.emergencyStop} disabled={!props.canCommand} onClick={()=>{disable();setStopStatus('Publishing emergency stop…');void props.onStop().then(()=>setStopStatus('Emergency stop published. Verify reported safety and disarmed hardware; publication is not acknowledgement.')).catch(e=>{setStopStatus('Stop outcome uncertain');setError((e as Error).message)})}}>Emergency stop</button>
 <p className={styles.support}>Emergency stop latches vehicle safety and disarms. Inspect the vehicle and reset safety from the native operator app before rearming.</p>
 {pendingToken&&<p role="status">Waiting for matching manual authority acknowledgement…</p>}
 {stopStatus&&<p role="status">{stopStatus}</p>}
 {error&&<p role="alert">{error}</p>}</section>
}
