import {fireEvent,render,screen} from '@testing-library/react';
import {expect,it,vi} from 'vitest';
import {LocalMap} from './LocalMap';
import type {Rover} from './generated';
const rovers=[{id:'1',frame_id:'shared',pose:{x:0,y:0,yaw:0},pose_age:0,hardware:{armed:true}},{id:'2',frame_id:'shared',pose:{x:10,y:5,yaw:1},pose_age:0,hardware:{armed:false}},{id:'3',frame_id:'shared',pose:null,pose_age:null}].map(r=>({...r,reference:null,goal:null})) as Rover[];
it('shows every rover together and selection does not hide other vehicles',()=>{
  const select=vi.fn();const draft=vi.fn();const props={rovers,selected:undefined,draft:null,onDraft:draft,onSelect:select};
  const{rerender}=render(<LocalMap {...props}/>);
  for(const id of ['1','2','3'])expect(screen.getByRole('button',{name:`Rover ${id}`})).toBeVisible();
  expect(screen.getByText('Position unknown')).toBeVisible();
  fireEvent.click(screen.getByRole('button',{name:'Rover 2'}));expect(select).toHaveBeenCalledWith('2');expect(draft).not.toHaveBeenCalled();
  rerender(<LocalMap {...props} selected={rovers[1]}/>);
  expect(screen.getByRole('button',{name:'Rover 1'})).toBeVisible();expect(screen.getByRole('button',{name:'Rover 2'})).toHaveAttribute('aria-pressed','true');
});
