import {useEffect,useMemo,useRef,useState} from 'react';
import type {Occupancy,Rover,Waypoint} from './generated';
import {roverPoint,sceneFrame,scenePoint,shortID} from './fleetScene';
import {markerOffsets} from './markerLayout';
import {fresh} from './geometry';
import {occupancyImage,occupancyTransform,occupancyCorners,validOccupancy} from './occupancy';
import styles from './App.module.css';
type Props={occupancy?:Occupancy|null;occupancyStatus?:string;rovers:Rover[];selected:Rover|undefined;draft:Waypoint|null;onDraft:(w:Waypoint)=>void;onSelect:(id:string)=>void};
export function LocalMap({rovers,selected,draft,onDraft,onSelect,occupancy=null,occupancyStatus=''}:Props){
  const svg=useRef<SVGSVGElement>(null);
  const [showOccupancy,setShowOccupancy]=useState(true);
  const grid=occupancy&&selected&&validOccupancy(occupancy,selected)?occupancy:null;
  const raster=useMemo(()=>grid?occupancyImage(grid):null,[grid?.cells,grid?.width,grid?.height]);
  const frame=useMemo(()=>sceneFrame(rovers),[rovers]);
  const [view,setView]=useState({u:0,v:0,span:50});
  const [size,setSize]=useState({width:1000,height:600});
  useEffect(()=>{if(!svg.current||typeof ResizeObserver==='undefined')return;const observer=new ResizeObserver(([entry])=>{if(entry.contentRect.width&&entry.contentRect.height)setSize({width:entry.contentRect.width,height:entry.contentRect.height})});observer.observe(svg.current);return()=>observer.disconnect()},[]);
  const fitted=useRef('');
  const drag=useRef<{x:number;y:number;u:number;v:number;moved:boolean}|null>(null);
  const placed=rovers.flatMap(r=>{const p=r.pose&&scenePoint(r.pose,r,frame);return p?[{rover:r,p}]:[]});
  const unplaced=rovers.filter(r=>!placed.some(p=>p.rover.id===r.id));
  const signature=`${frame.owner}:${frame.frameId}:${!!frame.reference}:${placed.map(p=>p.rover.id).join(',')}:${grid?.rover_id??''}:${grid?.run_id??''}`;
  function fit(){
    if(!placed.length){setView({u:0,v:0,span:50});return}
    const mapPoints=grid&&selected?occupancyCorners(grid).flatMap(p=>{const q=scenePoint(p,selected,frame);return q?[q]:[]}):[];
    const points=[...placed.map(p=>p.p),...mapPoints];
    const xs=points.map(v=>v.x),ys=points.map(v=>v.y);
    const minX=Math.min(...xs),maxX=Math.max(...xs),minY=Math.min(...ys),maxY=Math.max(...ys);
    setView({u:-(minY+maxY)/2,v:-(minX+maxX)/2,span:Math.max(grid?5:40,(maxY-minY)*1.5,(maxX-minX)*2.2)});
  }
  useEffect(()=>{if(fitted.current!==signature){fitted.current=signature;fit()}},[signature]);
  function world(x:number,y:number){const el=svg.current!;const matrix=el.getScreenCTM();if(!matrix)return null;const p=el.createSVGPoint();p.x=x;p.y=y;return p.matrixTransform(matrix.inverse())}
  const d=draft?.frame==='local'&&selected?scenePoint(draft,selected,frame):null;
  const placement=grid&&selected?occupancyTransform(grid,selected,frame):null;
  const {u,v,span}=view;
  const pixel=span/Math.min(size.width,size.height/.65);
  const offsets=markerOffsets(placed.map(({rover:r,p})=>({id:r.id,x:size.width/2+(-p.y-u)/pixel,y:size.height/2+(-p.x-v)/pixel})),size.width,size.height);
  return <div className={styles.scene}>
    <div className={styles.mapTools}><span className="yg-label">Shared fleet frame · metres</span><div className={styles.buttons}><button onClick={()=>setView(s=>({...s,span:Math.max(5,s.span/1.5)}))} aria-label="Zoom in">+</button><button onClick={()=>setView(s=>({...s,span:Math.min(40000,s.span*1.5)}))} aria-label="Zoom out">−</button><button onClick={fit}>Fit fleet</button></div></div>
    <div className={styles.occupancyStatus}><label><input type="checkbox" checked={showOccupancy} onChange={e=>setShowOccupancy(e.target.checked)}/> Occupancy map</label><span>{grid?(placement?`${grid.age>=2.5?'Stale map':'Observed map'} · ${grid.width} × ${grid.height} · ${grid.age.toFixed(1)} s old`:'Occupancy frame is unaligned'):occupancyStatus}</span></div>
    <div className={styles.localViewport}>
    <svg ref={svg} className={styles.localMap} viewBox={`${u-span/2} ${v-span*.325} ${span} ${span*.65}`} aria-label="Local rover map" role="group"
      onPointerDown={e=>{if((e.target as Element).closest('[data-rover-marker]'))return;drag.current={x:e.clientX,y:e.clientY,u,v,moved:false};e.currentTarget.setPointerCapture(e.pointerId)}}
      onPointerMove={e=>{const start=drag.current;if(!start)return;if(Math.hypot(e.clientX-start.x,e.clientY-start.y)>5)start.moved=true;if(start.moved){const scale=svg.current!.getScreenCTM()?.a??1;setView(s=>({...s,u:start.u-(e.clientX-start.x)/scale,v:start.v-(e.clientY-start.y)/scale}))}}}
      onPointerUp={e=>{const start=drag.current;drag.current=null;if(start&&!start.moved&&selected){const p=world(e.clientX,e.clientY);const target=p&&roverPoint({x:-p.y,y:-p.x},selected,frame);if(target)onDraft({frame:'local',x:Number(target.x.toFixed(2)),y:Number(target.y.toFixed(2)),yaw:null})}}}
      onPointerCancel={()=>{drag.current=null}}>
      <defs><pattern id="metre-grid" width="5" height="5" patternUnits="userSpaceOnUse"><path d="M 5 0 L 0 0 0 5" fill="none" stroke="var(--rule)" strokeWidth={span/1000}/></pattern></defs>
      <rect x={u-span/2} y={v-span*.325} width={span} height={span*.65} fill="url(#metre-grid)"/>
      {showOccupancy&&grid&&raster&&placement&&<image aria-label="Observed occupancy grid" href={raster} width={grid.width} height={grid.height} transform={`matrix(${placement.join(' ')})`} opacity={grid.age>=2.5?0.45:1} style={{imageRendering:'pixelated',pointerEvents:'none'}}/>}
      {placed.map(({rover:r,p})=>{
        const goal=r.goal&&r.goal.state!=='idle'?scenePoint(r.goal,r,frame):null;
        const active=selected?.id===r.id;
        const offset=offsets[r.id]??{x:0,y:0};
        return <g key={r.id}>
          {goal&&<g><path d={`M ${-p.y} ${-p.x} L ${-goal.y} ${-goal.x}`} stroke="var(--graphite)" strokeWidth={span/800} strokeDasharray={`${span/100} ${span/100}`}/><circle cx={-goal.y} cy={-goal.x} r={span/100} fill="var(--paper)" stroke="var(--ink)" strokeWidth={span/700}/></g>}
          {(offset.x!==0||offset.y!==0)&&<g><path d={`M ${-p.y} ${-p.x} l ${offset.x*pixel} ${offset.y*pixel}`} stroke="var(--graphite)" strokeWidth={pixel}/><circle cx={-p.y} cy={-p.x} r={2.5*pixel} fill="var(--ink)"/></g>}
          <g data-rover-marker={r.id} role="button" tabIndex={0} aria-label={`Rover ${r.id}`} aria-pressed={active} onClick={e=>{e.stopPropagation();onSelect(r.id)}} onKeyDown={e=>{if(e.key==='Enter'||e.key===' '){e.preventDefault();onSelect(r.id)}}} className={styles.sceneMarker} transform={`translate(${-p.y+offset.x*pixel} ${-p.x+offset.y*pixel})`}>
            <title>Rover {r.id} · {r.membership}{!fresh(r.pose_age)?' · Last known position':''}</title>
            <circle r={22*pixel} fill="var(--paper)" stroke={active?'var(--ink)':'var(--rule)'} strokeWidth={span/600}/>
            <g transform={`rotate(${-(r.pose!.yaw+(frame.reference?(r.reference?.rotation??0):0))*180/Math.PI})`}><path d={`M 0 ${-span/60} L ${span/85} ${span/80} L 0 ${span/140} L ${-span/85} ${span/80} Z`} fill="var(--ink)"/></g>
            <text y={38*pixel} textAnchor="middle" fontSize={14*pixel} fill="var(--ink)">Rover {shortID(r.id)}</text>
            <text y={54*pixel} textAnchor="middle" fontSize={12*pixel} fill="var(--graphite)">{!fresh(r.pose_age)?'Last known':r.hardware?.armed?'Armed':'Disarmed'}</text>
          </g>
        </g>
      })}
      {d&&<g stroke="var(--ink)" strokeWidth={span/600}><path d={`M ${-d.y-span/70} ${-d.x} H ${-d.y+span/70} M ${-d.y} ${-d.x-span/70} V ${-d.x+span/70}`}/><circle cx={-d.y} cy={-d.x} r={span/45} fill="none" strokeDasharray={`${span/150} ${span/150}`}/></g>}
    </svg>
    {!!unplaced.length&&<div className={styles.unlocated} aria-label="Vehicles without a shared position"><span className="yg-label">Position unavailable</span><div>{unplaced.map(r=><button key={r.id} aria-label={`Rover ${r.id}`} aria-pressed={selected?.id===r.id} onClick={()=>onSelect(r.id)}><strong>Rover {shortID(r.id)}</strong><small>{r.pose?'Frame unaligned':'Position unknown'}</small></button>)}</div></div>}
    </div>
    <p className={styles.occupancyLegend}>Occupancy: white free · black occupied · gray unknown. Selected rover only.</p>
    <div className={styles.mapCaption}><span>North ↑ · West ← · {span.toFixed(0)} m across</span><span>{placed.length} positioned / {rovers.length} vehicles</span></div>
  </div>
}
