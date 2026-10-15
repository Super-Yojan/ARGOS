import type {Reference,Rover} from './generated';
import {geographic,local} from './geometry';
export type Point={x:number;y:number};
export type SceneFrame={reference:Reference|null;frameId:string|null;owner:string};
export function sceneFrame(rovers:Rover[]):SceneFrame{
  const anchor=rovers.find(r=>r.pose&&r.reference)??rovers.find(r=>r.pose);
  return {reference:anchor?.reference?{...anchor.reference,anchor_x:0,anchor_y:0,rotation:0}:null,frameId:anchor?.frame_id??null,owner:anchor?.id??''};
}
export function scenePoint(p:Point,r:Rover,f:SceneFrame):Point|null{
  if(f.reference&&r.reference)return local(geographic(p,r.reference),f.reference);
  if(!f.reference&&(r.id===f.owner||(f.frameId!==null&&r.frame_id===f.frameId)))return p;
  return null;
}
export function roverPoint(p:Point,r:Rover,f:SceneFrame):Point|null{
  if(f.reference&&r.reference)return local(geographic(p,f.reference),r.reference);
  if(!f.reference&&(r.id===f.owner||(f.frameId!==null&&r.frame_id===f.frameId)))return p;
  return null;
}
export function shortID(id:string){return id.length>8?`…${id.slice(-6)}`:id}
