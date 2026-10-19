import { useEffect, useRef, useState } from 'react';
import type { Feature } from 'geojson';
import { Map as MapLibreMap, setWorkerUrl, Marker, type GeoJSONSource, type StyleSpecification } from 'maplibre-gl';
import 'maplibre-gl/dist/maplibre-gl.css';
import workerURL from 'maplibre-gl/dist/maplibre-gl-worker.mjs?worker&url';
import type { Occupancy, Rover, Waypoint } from './generated';
import { markerOffsets } from './markerLayout';
import { shortID } from './fleetScene';
import styles from './App.module.css';
setWorkerUrl(workerURL);
const streetMap:StyleSpecification={version:8,sources:{streets:{type:'raster',tiles:['https://tile.openstreetmap.org/{z}/{x}/{y}.png'],tileSize:256,maxzoom:19,attribution:'© <a href="https://www.openstreetmap.org/copyright">OpenStreetMap</a> contributors'}},layers:[{id:'streets',type:'raster',source:'streets',paint:{'raster-saturation':-1}}]};
type Props={rovers:Rover[];selected:Rover|undefined;styleURL:string|null;onLocal:()=>void;onSelect?:(id:string)=>void;connected?:boolean;draft?:Waypoint|null;onDraft?:(w:Waypoint)=>void;occupancy?:Occupancy|null;occupancyStatus?:string};
function hasGPS(r:Rover){const g=r.gps;return !!g&&Number.isFinite(g.latitude)&&Math.abs(g.latitude)<=85&&Number.isFinite(g.longitude)&&Math.abs(g.longitude)<=180&&Number.isFinite(g.accuracy)&&g.accuracy>=0&&Number.isFinite(g.age)&&g.age>=0&&g.age<=3}
export default function GeographicMap({rovers,selected,styleURL,onLocal,onSelect=()=>{},connected=true}:Props){
 const visible=connected?rovers.filter(hasGPS):[];
 const initial=visible[0]?.gps;
 const container=useRef<HTMLDivElement>(null),map=useRef<MapLibreMap|null>(null);
 const markers=useRef(new Map<string,Marker>()),fitted=useRef('');
 const onSelectRef=useRef(onSelect);onSelectRef.current=onSelect;
 const [loaded,setLoaded]=useState(false),[failed,setFailed]=useState(false),[viewport,setViewport]=useState(0);
 useEffect(()=>{
  setFailed(false);
  if(!initial||!container.current)return;
  try{
   const m=new MapLibreMap({container:container.current,style:styleURL??streetMap,center:[initial.longitude,initial.latitude],zoom:17,attributionControl:{compact:false},transformRequest:url=>({url,referrerPolicy:'strict-origin-when-cross-origin'})});
   map.current=m;
   const observer=typeof ResizeObserver==='undefined'?null:new ResizeObserver(()=>{m.resize();fitted.current='';setViewport(v=>v+1)});
   observer?.observe(container.current);
   m.on('load',()=>{
    m.addSource('rovers',{type:'geojson',data:{type:'FeatureCollection',features:[]}});
    m.addLayer({id:'callout-connectors',type:'line',source:'rovers',filter:['==',['get','kind'],'connector'],paint:{'line-color':'#525252','line-width':1}});
    m.addLayer({id:'gps-marks',type:'circle',source:'rovers',filter:['==',['get','kind'],'rover'],paint:{'circle-radius':6,'circle-color':'#000000','circle-stroke-color':'#FFFFFF','circle-stroke-width':1}});
    setLoaded(true);
   });
   m.on('error',()=>setFailed(true));m.on('move',()=>setViewport(v=>v+1));
   return()=>{observer?.disconnect();for(const marker of markers.current.values())marker.remove();markers.current.clear();fitted.current='';m.remove();map.current=null;setLoaded(false)};
  }catch{setFailed(true)}
 },[styleURL,!!initial]);
 useEffect(()=>{
  const m=map.current;if(!m||!loaded)return;
  const offsets=markerOffsets(visible.map(r=>{const p=m.project([r.gps!.longitude,r.gps!.latitude]);return{id:r.id,x:p.x,y:p.y}}),container.current?.clientWidth||1000,container.current?.clientHeight||600);
  for(const[id,marker]of markers.current)if(!visible.some(r=>r.id===id)){marker.remove();markers.current.delete(id)}
  const features:Feature[]=[];
  for(const r of visible){
   const g=r.gps!;let marker=markers.current.get(r.id);
   if(!marker){const button=document.createElement('button');button.className=styles.geoRoverMarker;button.setAttribute('aria-label',`Rover ${r.id}`);button.onclick=e=>{e.stopPropagation();onSelectRef.current(r.id)};marker=new Marker({element:button}).setLngLat([g.longitude,g.latitude]).addTo(m);markers.current.set(r.id,marker)}
   marker.setLngLat([g.longitude,g.latitude]).setOffset([offsets[r.id].x,offsets[r.id].y]);
   const button=marker.getElement(),label=document.createElement('strong'),detail=document.createElement('small');
   label.textContent=`Rover ${shortID(r.id)}`;detail.textContent=`GPS ±${g.accuracy.toFixed(1)} m · ${g.age.toFixed(1)} s`;button.replaceChildren(label,detail);button.setAttribute('aria-pressed',String(selected?.id===r.id));
   features.push({type:'Feature',geometry:{type:'Point',coordinates:[g.longitude,g.latitude]},properties:{kind:'rover',id:r.id}});
   const offset=offsets[r.id];if(offset.x||offset.y){const p=m.project([g.longitude,g.latitude]),end=m.unproject([p.x+offset.x,p.y+offset.y]);features.push({type:'Feature',geometry:{type:'LineString',coordinates:[[g.longitude,g.latitude],[end.lng,end.lat]]},properties:{kind:'connector'}})}
  }
  (m.getSource('rovers') as GeoJSONSource).setData({type:'FeatureCollection',features});
  const signature=visible.map(r=>r.id).join(',');
  if(signature&&signature!==fitted.current){fitted.current=signature;fitFleet()}
 },[rovers,connected,loaded,selected?.id,viewport]);
 function fitFleet(){if(!visible.length||!map.current)return;const lat=visible.map(r=>r.gps!.latitude),lon=visible.map(r=>r.gps!.longitude);map.current.fitBounds([[Math.min(...lon),Math.min(...lat)],[Math.max(...lon),Math.max(...lat)]],{padding:100,maxZoom:18,duration:0})}
 const missing=rovers.filter(r=>!connected||!hasGPS(r));
 return <div className={styles.geoWrap}>
  <div className={styles.mapTools}><span className="yg-label">WGS84 · raw GPS</span><button disabled={!loaded||!visible.length} onClick={fitFleet}>Fit fleet</button></div>
  <div ref={container} className={styles.geoMap}/>
  {(!visible.length||failed)&&<div className={styles.mapNotice} role="status"><strong>{failed?'Basemap unavailable':'GPS position unavailable'}</strong><p>{!connected?'Live connection required for current GPS.':!visible.length?'Waiting for a fresh GPS fix.':'Basemap could not be loaded. GPS coordinates remain available in rover details.'}</p><button onClick={onLocal}>Use local map</button></div>}
  {!!missing.length&&<div className={styles.unlocated} aria-label="Vehicles without a GPS fix"><span className="yg-label">GPS unavailable</span><div>{missing.map(r=><button key={r.id} aria-label={`Rover ${r.id}`} onClick={()=>onSelect(r.id)}><strong>Rover {shortID(r.id)}</strong><small>Waiting for fresh GPS</small></button>)}</div></div>}
  <p className={styles.mapCaption}>Reported GPS fixes with measured accuracy. Compass and local alignment are not required. Waypoint commands use the local map; occupancy stays in the local frame.</p>
 </div>
}
