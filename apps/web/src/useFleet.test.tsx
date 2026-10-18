import {act,renderHook} from '@testing-library/react';
import {afterEach,beforeEach,describe,expect,it,vi} from 'vitest';
import {useFleet} from './useFleet';
import type {Identity} from './generated';
class Events {
 static instances:Events[]=[];
 onerror:(()=>void)|null=null;
 listener:((event:{data:string})=>void)|undefined;
 close=vi.fn();
 constructor(){Events.instances.push(this)}
 addEventListener(_name:string,listener:(event:{data:string})=>void){this.listener=listener}
 snapshot(){this.listener?.({data:JSON.stringify({link:'healthy',rovers:[],operations:[],claim:null,now:1})})}
}
const user:Identity={id:'1',login:'test',role:'operator',csrf:'csrf'};
beforeEach(()=>{vi.useFakeTimers();Events.instances=[];vi.stubGlobal('EventSource',Events)});
afterEach(()=>{vi.useRealTimers();vi.unstubAllGlobals()});
describe('fleet stream recovery',()=>{
 it('reopens a stalled connection and enables commands only after a new snapshot',()=>{
  const clear=vi.fn();const {result}=renderHook(()=>useFleet(user,clear));
  act(()=>Events.instances[0].snapshot());expect(result.current.connected).toBe(true);
  act(()=>vi.advanceTimersByTime(4500));
  expect(result.current.connected).toBe(false);expect(clear).toHaveBeenCalledOnce();
  expect(Events.instances[0].close).toHaveBeenCalled();expect(Events.instances).toHaveLength(2);
  act(()=>Events.instances[1].snapshot());expect(result.current.connected).toBe(true);
 });
 it('retries a stream that never delivered its first snapshot',()=>{
  const clear=vi.fn();
  renderHook(()=>useFleet(user,clear));act(()=>vi.advanceTimersByTime(4500));expect(Events.instances).toHaveLength(2);
 });
 it('ignores late events from a replaced stream and closes all work on unmount',()=>{
  const clear=vi.fn();
  const {result,unmount}=renderHook(()=>useFleet(user,clear));act(()=>vi.advanceTimersByTime(4500));
  act(()=>Events.instances[0].snapshot());expect(result.current.connected).toBe(false);
  unmount();act(()=>vi.advanceTimersByTime(20000));expect(Events.instances).toHaveLength(2);expect(Events.instances[1].close).toHaveBeenCalled();
 });
});
