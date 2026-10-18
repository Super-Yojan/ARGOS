import {expect,it} from 'vitest';
import {sceneFrame,scenePoint,roverPoint} from './fleetScene';
import type {Rover} from './generated';
function rover(id:string,frame:string,x:number):Rover{return {id,pose:{x,y:0,yaw:0},frame_id:frame,reference:null} as Rover}
it('shows independent phone frames together only through reported alignment',()=>{
  const a=rover('1','phone-a',0),b=rover('2','phone-b',0);a.reference={latitude:38,longitude:-77,anchor_x:0,anchor_y:0,rotation:0,frame_id:'phone-a'};b.reference={...a.reference,latitude:38.0001,frame_id:'phone-b'};
  const frame=sceneFrame([a,b]);const p=scenePoint(b.pose!,b,frame)!;expect(p.x).toBeGreaterThan(10);expect(scenePoint(a.pose!,a,frame)!.x).toBeCloseTo(0);expect(roverPoint(p,b,frame)!.x).toBeCloseTo(0);
});
it('shares reported local frames but never overlaps unrelated local origins',()=>{
  const a=rover('1','shared',0),b=rover('2','shared',10),c=rover('3','other',0);const frame=sceneFrame([a,b,c]);expect(scenePoint(b.pose!,b,frame)?.x).toBe(10);expect(scenePoint(c.pose!,c,frame)).toBeNull();
});
it('does not assume missing frame identifiers mean a shared coordinate system',()=>{
  const a=rover('1','a',0),b=rover('2','b',0);a.frame_id=null;b.frame_id=null;const frame=sceneFrame([a,b]);expect(scenePoint(a.pose!,a,frame)).not.toBeNull();expect(scenePoint(b.pose!,b,frame)).toBeNull();
});
