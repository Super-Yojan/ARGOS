import { describe,expect,it } from 'vitest';
import { geographic,local,validWaypoint,ready,readinessProblems } from './geometry';
import type { Reference,Rover } from './generated';
const ref:Reference={latitude:38.8297,longitude:-77.3075,anchor_x:12,anchor_y:-5,rotation:Math.PI/3,frame_id:'a'};
describe('map coordinates',()=>{
  it('round trips a rotated, translated phone frame',()=>{const c=geographic({x:30,y:10},ref);const p=local(c,ref);expect(p.x).toBeCloseTo(30,6);expect(p.y).toBeCloseTo(10,6)});
  it('north increases latitude and west decreases longitude',()=>{const a={...ref,anchor_x:0,anchor_y:0,rotation:0};const c=geographic({x:5,y:5},a);expect(c.latitude).toBeGreaterThan(a.latitude);expect(c.longitude).toBeLessThan(a.longitude)});
  it('rejects NaN, coordinate limits and nonfinite yaw',()=>{expect(validWaypoint({frame:'local',x:NaN,y:0,yaw:null})).toBe(false);expect(validWaypoint({frame:'local',x:20001,y:0,yaw:null})).toBe(false);expect(validWaypoint({frame:'geographic',latitude:86,longitude:0,yaw:null})).toBe(false);expect(validWaypoint({frame:'local',x:1,y:0,yaw:Infinity})).toBe(false)});
  it('unknown pose never becomes a ready rover at the origin',()=>{const rover={membership:'online',pose:null,pose_age:0,hardware:{ready:true,armed:true,age:0},authority:{age:0,run_id:'a',safety:'clear'}} as Rover;expect(ready(rover)).toBe(false)});
});
describe('readiness explanations',()=>{
 it('identifies a stale map without blaming a fresh pose',()=>{
  const r={membership:'online',pose:{x:0,y:0,yaw:0},pose_age:0.1,hardware:{ready:true,armed:false,age:0.1},authority:{run_id:'r',age:0.1,safety:'hold',reason:'map_stale'}} as Rover;
  expect(readinessProblems(r).join('; ')).toContain('Obstacle map is missing or stale');
  expect(readinessProblems(r).join('; ')).toContain('Vehicle is disarmed');
  expect(readinessProblems(r).join('; ')).not.toMatch(/pose/i);
  expect(readinessProblems(r,true)).toEqual(['Safety hold: Obstacle map is missing or stale']);
 });
});
