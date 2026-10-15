import {describe,it,expect} from 'vitest';
import {occupancyPixels,occupancyTransform,validOccupancy} from './occupancy';
import type {Occupancy,Rover} from './generated';
const grid:Occupancy={rover_id:'7',run_id:'run',sequence:'9007199254740993',frame_id:'frame',width:2,height:2,resolution:0.5,origin_x:1,origin_y:2,cells:[-1,0,64,65],age:0};
const rover={id:'7',frame_id:'frame',authority:{run_id:'run'},reference:null} as Rover;
describe('occupancy rendering',()=>{
 it('keeps unknown gray, free white and occupied black at the Swift threshold',()=>{expect([...occupancyPixels(grid)]).toEqual([180,180,180,180,255,255,255,235,255,255,255,235,0,0,0,220])});
 it('projects columns north and rows west into the local fleet frame',()=>{expect(occupancyTransform(grid,rover,{owner:'7',frameId:'frame',reference:null})).toEqual([0,-0.5,-0.5,0,-2,-1])});
 it('rejects another rover, run, frame, malformed or oversized grids',()=>{expect(validOccupancy(grid,rover)).toBe(true);for(const bad of [{...grid,run_id:'old'},{...grid,frame_id:'old'},{...grid,rover_id:'8'},{...grid,cells:[-1]},{...grid,width:250001},{...grid,resolution:NaN}])expect(validOccupancy(bad,rover)).toBe(false)});
});
