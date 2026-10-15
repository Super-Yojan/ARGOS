import {useEffect,useState} from 'react';
import type {Identity,Occupancy,Rover} from './generated';
import {api} from './api';
import {validOccupancy} from './occupancy';
export function useOccupancy(user:Identity|null,rover:Rover|undefined,connected:boolean){
 const [value,setValue]=useState<{map:Occupancy;received:number}|null>(null),[status,setStatus]=useState(''),[now,setNow]=useState(Date.now());
 const id=rover?.id,run=rover?.authority?.run_id,frame=rover?.frame_id;
 useEffect(()=>{setValue(null);setStatus(id?'Waiting for occupancy grid…':'Select a rover to view its occupancy map.')},[id,run,frame]);
 useEffect(()=>{
  if(!user||!rover||!connected)return;
  let stopped=false;let timer:ReturnType<typeof setTimeout>;
  async function poll(){
   try{
    const map=await api<Occupancy|null>(`/rovers/${id}/occupancy`);
    if(stopped)return;
    if(map&&validOccupancy(map,rover!)){setValue(previous=>({map:previous&&previous.map.run_id===map.run_id&&previous.map.sequence===map.sequence&&previous.map.frame_id===map.frame_id?{...map,cells:previous.map.cells}:map,received:Date.now()}));setStatus('')}
    else {setValue(null);setStatus(map?'Occupancy grid does not match this rover frame or has unsupported dimensions.':'No occupancy grid received from Terra.')}
   }catch(e){if(!stopped)setStatus(`Occupancy request failed: ${e instanceof Error?e.message:'Connection unavailable'}`)}
   if(!stopped)timer=setTimeout(()=>void poll(),1000);
  }
  void poll();return()=>{stopped=true;clearTimeout(timer)};
 },[user,id,run,frame,connected]);
 useEffect(()=>{const timer=setInterval(()=>setNow(Date.now()),500);return()=>clearInterval(timer)},[]);
 const map=value&&rover&&validOccupancy(value.map,rover)?{...value.map,age:value.map.age+Math.max(0,now-value.received)/1000}:null;
 return {map,status};
}
