import {fireEvent,render,screen,act} from '@testing-library/react';
import {it,expect,vi} from 'vitest';
import {DrivePanel} from './DrivePanel';
import type {Rover} from './generated';
const rover:Rover={id:'7',membership:'online',pose:{x:0,y:0,yaw:0},pose_age:0,goal:null,goal_age:null,goal_observation:null,frame_id:'f',authority:{run_id:'r',revision:'1',requested_level:'teleop',supported_levels:['teleop'],safety:'clear',age:0,token:'ack',result:'accepted'},hardware:{ready:true,armed:true,reason:'Ready',age:0},reference:null};
it('requires explicit takeover and stops on release and focus loss',async()=>{
 vi.useFakeTimers();const drive=vi.fn().mockResolvedValue(undefined),take=vi.fn().mockResolvedValue(undefined);
 render(<DrivePanel rover={rover} canCommand now={1} onDrive={drive} onTakeover={take} onStop={vi.fn()}/>);
 fireEvent.keyDown(window,{key:'w'});expect(drive).not.toHaveBeenCalled();
 await act(async()=>{fireEvent.click(screen.getByRole('button',{name:'Take over manual driving'}));});
 fireEvent.keyDown(window,{key:'w'});await act(async()=>{vi.advanceTimersByTime(200)});expect(drive.mock.calls.at(-1)?.[0].linear).toBe(0.3);
 fireEvent.keyUp(window,{key:'w'});expect(drive.mock.calls.at(-1)?.[0].linear).toBe(0);
 fireEvent.keyDown(window,{key:'w'});fireEvent.blur(window);expect(drive.mock.calls.at(-1)?.[0].linear).toBe(0);
 await act(async()=>{vi.advanceTimersByTime(1000)});expect(drive.mock.calls.at(-1)?.[0].linear).toBe(0);vi.useRealTimers();
});
it('never resumes held input after a disconnect',async()=>{const drive=vi.fn().mockResolvedValue(undefined),props={rover,now:1,onDrive:drive,onTakeover:vi.fn().mockResolvedValue(undefined),onStop:vi.fn()};const {rerender}=render(<DrivePanel {...props} canCommand/>);await act(async()=>{fireEvent.click(screen.getByRole('button',{name:'Take over manual driving'}));});fireEvent.pointerDown(screen.getByRole('button',{name:'Forward'}));rerender(<DrivePanel {...props} canCommand={false}/>);rerender(<DrivePanel {...props} canCommand/>);expect(screen.getByRole('button',{name:'Forward'})).toBeDisabled();expect(drive.mock.calls.at(-1)?.[0].linear).toBe(0);});
it('waits for the matching takeover acknowledgement and invalidates delayed responses on blur',async()=>{
 const drive=vi.fn().mockResolvedValue(undefined);const other={...rover,authority:{...rover.authority!,requested_level:'waypoint'}};
 const props={canCommand:true,now:1,onDrive:drive,onStop:vi.fn(),onTakeover:vi.fn().mockResolvedValue('take-token')};
 const {rerender}=render(<DrivePanel {...props} rover={other}/>);
 await act(async()=>{fireEvent.click(screen.getByRole('button',{name:'Take over manual driving'}))});expect(screen.getByRole('button',{name:'Forward'})).toBeDisabled();
 rerender(<DrivePanel {...props} rover={{...rover,authority:{...rover.authority!,revision:'2',token:'take-token'}}}/>);expect(screen.getByRole('button',{name:'Forward'})).toBeEnabled();
 fireEvent.blur(window);expect(screen.getByRole('button',{name:'Forward'})).toBeDisabled();
 let resolve!:(v:string)=>void;rerender(<DrivePanel {...props} rover={rover} onTakeover={()=>new Promise(r=>{resolve=r})}/>);
 fireEvent.click(screen.getByRole('button',{name:'Take over manual driving'}));fireEvent.blur(window);
 await act(async()=>resolve('ack'));expect(screen.getByRole('button',{name:'Forward'})).toBeDisabled();
});
it('accepts driving keys while the takeover button retains focus',async()=>{vi.useFakeTimers();const drive=vi.fn().mockResolvedValue(undefined);render(<DrivePanel rover={rover} canCommand now={1} onDrive={drive} onTakeover={vi.fn().mockResolvedValue(undefined)} onStop={vi.fn()}/>);const button=screen.getByRole('button',{name:'Take over manual driving'});await act(async()=>fireEvent.click(button));fireEvent.keyDown(button,{key:'w'});await act(async()=>vi.advanceTimersByTime(200));expect(drive.mock.calls.at(-1)?.[0].linear).toBe(0.3);fireEvent.keyUp(button,{key:'w'});vi.useRealTimers();});
