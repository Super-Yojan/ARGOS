import type { Reference,Rover,Waypoint } from './generated';
const scale=40_075_016.686/360;
export function geographic(p:{x:number;y:number},r:Reference){
  const x=p.x-r.anchor_x,y=p.y-r.anchor_y;
  const north=Math.cos(r.rotation)*x-Math.sin(r.rotation)*y;
  const west=Math.sin(r.rotation)*x+Math.cos(r.rotation)*y;
  return {latitude:r.latitude+north/scale,longitude:r.longitude-west/(scale*Math.cos(r.latitude*Math.PI/180))};
}
export function local(p:{latitude:number;longitude:number},r:Reference){
  const x=(p.latitude-r.latitude)*scale,y=-(p.longitude-r.longitude)*scale*Math.cos(r.latitude*Math.PI/180);
  return {x:r.anchor_x+Math.cos(r.rotation)*x+Math.sin(r.rotation)*y,y:r.anchor_y-Math.sin(r.rotation)*x+Math.cos(r.rotation)*y};
}
export function fresh(age:number|null){return age!==null&&Number.isFinite(age)&&age>=0&&age<90}
export function ready(r:Rover){return r.membership==='online'&&!!r.pose&&fresh(r.pose_age)&&!!r.hardware?.ready&&r.hardware.armed&&fresh(r.hardware.age)&&!!r.authority?.run_id&&fresh(r.authority.age)&&['clear','active'].includes(r.authority.safety)}
export function validWaypoint(w:Waypoint){
  if(w.yaw!==null&&(!Number.isFinite(w.yaw)||Math.abs(w.yaw)>2*Math.PI))return false;
  return w.frame==='local'?Number.isFinite(w.x)&&Math.abs(w.x)<=20000&&Number.isFinite(w.y)&&Math.abs(w.y)<=20000:Number.isFinite(w.latitude)&&Math.abs(w.latitude)<=85&&Number.isFinite(w.longitude)&&Math.abs(w.longitude)<=180;
}
export function ageLabel(age:number|null){return age===null?'Unknown':`${age.toFixed(1)} s ago`}
export function terminal(phase:string){return ['confirmed','arrived','cancelled','rejected','unconfirmed','interrupted'].includes(phase)}
export function safetyReason(reason:string|null|undefined){
  const descriptions:Record<string,string>={map_stale:'Obstacle map is missing or stale',sensor_unhealthy:'Phone tracking or sensor health is not ready',operator_stop:'Emergency stop is active',supervision_paused:'Supervision is paused',obstacle_blocked:'Route is blocked by an obstacle',idle:'Idle',active:'Active'};
  return reason?(descriptions[reason]??reason.replaceAll('_',' ')):'';
}
export function readinessProblems(r:Rover,arming=false){
 const problems:string[]=[];const limit=arming?0.5:90;
 const recent=(age:number|null|undefined)=>age!=null&&Number.isFinite(age)&&age>=0&&age<limit;
 if(r.membership!=='online')problems.push('Vehicle is offline');
 if(!r.pose)problems.push('Pose is unavailable');else if(!recent(r.pose_age))problems.push(`Pose update is stale (${ageLabel(r.pose_age)})`);
 if(!r.hardware)problems.push('Hardware status is unavailable');else{
  if(!r.hardware.ready)problems.push(`Hardware is not ready${r.hardware.reason?`: ${r.hardware.reason}`:''}`);
  if(!recent(r.hardware.age))problems.push(`Hardware status is stale (${ageLabel(r.hardware.age)})`);
  if(!arming&&!r.hardware.armed)problems.push('Vehicle is disarmed');
 }
 const a=r.authority;
 if(!a?.run_id)problems.push('Vehicle authority is unavailable');else{
  if(!recent(a.age))problems.push(`Authority update is stale (${ageLabel(a.age)})`);
  if(!(arming?['clear']:['clear','active']).includes(a.safety))problems.push(`Safety ${a.safety.replaceAll('_',' ')}${a.reason?`: ${safetyReason(a.reason)}`:''}`);
 }
 return problems;
}
