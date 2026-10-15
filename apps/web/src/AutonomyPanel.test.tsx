import {fireEvent,render,screen,waitFor} from '@testing-library/react';
import {describe,it,expect,vi} from 'vitest';
import {AutonomyPanel} from './AutonomyPanel';
import type {Rover} from './generated';
const rover={id:'7',membership:'online',pose:{x:0,y:0,yaw:0},pose_age:0.1,authority:{run_id:'r',revision:'1',requested_level:'waypoint',supported_levels:['teleop','waypoint_direct','waypoint'],age:0.1,safety:'hold',reason:'map_stale'},hardware:{ready:true,armed:false,age:0.1}} as Rover;
describe('autonomy levels',()=>{
 it('matches Swift levels, disables unsupported choices, and requests a supported mode without arming',async()=>{const send=vi.fn().mockResolvedValue(undefined);render(<AutonomyPanel rover={rover} canCommand={true} onRequest={send}/>);expect(screen.getByRole('button',{name:'L5 · Decision making'})).toBeDisabled();expect(screen.getByRole('button',{name:'L1 · Assisted teleop'})).toBeDisabled();expect(send).not.toHaveBeenCalled();fireEvent.click(screen.getByRole('button',{name:'L2 · Waypoint · no avoidance'}));await waitFor(()=>expect(send).toHaveBeenCalledWith('waypoint_direct','r','1'));expect(screen.getByText(/Obstacle map is missing or stale/)).toBeVisible();});
 it('requires control and fresh authority to change mode',()=>{const {rerender}=render(<AutonomyPanel rover={rover} canCommand={false} onRequest={vi.fn()}/>);expect(screen.getByRole('button',{name:'L0 · Fully teleop'})).toBeDisabled();rerender(<AutonomyPanel rover={{...rover,authority:{...rover.authority!,age:1}}} canCommand={true} onRequest={vi.fn()}/>);expect(screen.getByRole('button',{name:'L0 · Fully teleop'})).toBeDisabled();});
});
