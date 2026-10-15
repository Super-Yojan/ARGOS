import { fireEvent,render,screen,waitFor } from '@testing-library/react';
import { describe,expect,it,vi } from 'vitest';
import { useState } from 'react';
import { WaypointPanel } from './WaypointPanel';
import type { Rover,Waypoint } from './generated';
const rover:Rover={id:'18446744073709551615',membership:'online',pose:{x:0,y:0,yaw:0},pose_age:0,goal:null,goal_age:null,goal_observation:null,frame_id:'a',authority:{run_id:'r',revision:'1',requested_level:'waypoint',supported_levels:['waypoint'],safety:'clear',age:0,token:null,result:null},hardware:{ready:true,armed:true,reason:'Simulated',age:0},reference:null};
function Harness({send=vi.fn<(_w:Waypoint,_r:string)=>Promise<void>>(),connected=true,canCommand=true}:{send?:(_w:Waypoint,_r:string)=>Promise<void>;connected?:boolean;canCommand?:boolean}){const[draft,setDraft]=useState<Waypoint|null>(null);return <WaypointPanel rover={rover} frame="local" draft={draft} onDraft={setDraft} canCommand={canCommand} connected={connected} operation={undefined} onSend={send} onCancel={vi.fn()}/>}
describe('waypoint confirmation',()=>{
  it('never sends when coordinates are entered or reviewed; sends only on confirmation',async()=>{
    const send=vi.fn<(_w:Waypoint,_r:string)=>Promise<void>>().mockResolvedValue(undefined);render(<Harness send={send}/>);
    fireEvent.change(screen.getByLabelText('North / x (m)'),{target:{value:'12'}});fireEvent.change(screen.getByLabelText('West / y (m)'),{target:{value:'3'}});
    expect(send).not.toHaveBeenCalled();fireEvent.click(screen.getByText('Review waypoint'));
    await waitFor(()=>expect(screen.getByText('Send waypoint')).toBeEnabled());expect(send).not.toHaveBeenCalled();
    fireEvent.click(screen.getByText('Send waypoint'));await waitFor(()=>expect(send).toHaveBeenCalledOnce());
    expect(send.mock.calls[0][0]).toEqual({frame:'local',x:12,y:3,yaw:null});expect(send.mock.calls[0][1]).toMatch(/^[a-f0-9-]{36}$/);
  });
  it('rejects empty coordinate inputs instead of treating them as zero',()=>{render(<Harness/>);fireEvent.click(screen.getByText('Review waypoint'));expect(screen.getByRole('alert')).toHaveTextContent('finite coordinates')});
  it('disables cancellation without current operator control',()=>{render(<Harness canCommand={false}/>);expect(screen.getByText('Cancel rover goal')).toBeDisabled()});
});
