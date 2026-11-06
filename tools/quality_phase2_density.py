"""Independent decoded-DST local sewn-length and puncture measurements."""
from collections import Counter, defaultdict
import math
from pathlib import Path
import json

import pyembroidery as pe
import quality_phase2_validation as q


def measure(folder):
    plan=json.loads((folder/"design.plan.json").read_text(encoding="utf-8"))
    roles=[m["role"] for s,m in zip(plan["stitches"],plan["metadata"],strict=True) if s["command"]=="Stitch"]
    records=pe.read(str(folder/"design.dst")).stitches
    lengths=defaultdict(Counter)
    punctures=defaultdict(Counter)
    previous=(0.,0.)
    index=0
    for x,y,code in records:
        point=(x/10,y/10)
        flag=pe.decode_embroidery_command(code)[0]
        if flag==pe.STITCH:
            role=roles[index]
            index+=1
            punctures[(math.floor(point[0]),math.floor(point[1]))][role]+=1
            dx,dy=point[0]-previous[0],point[1]-previous[1]
            cuts={0.,1.}
            for axis,delta in ((0,dx),(1,dy)):
                if abs(delta)>1e-12:
                    low,high=sorted((previous[axis],point[axis]))
                    for boundary in range(math.floor(low)+1,math.ceil(high)):
                        t=(boundary-previous[axis])/delta
                        if 0<t<1:
                            cuts.add(t)
            total=math.hypot(dx,dy)
            cuts=sorted(cuts)
            for lo,hi in zip(cuts,cuts[1:]):
                t=(lo+hi)/2
                cell=(math.floor(previous[0]+t*dx),math.floor(previous[1]+t*dy))
                lengths[cell][role]+=(hi-lo)*total
        if flag in (pe.STITCH,pe.JUMP):
            previous=point
    if index != len(roles):
        raise AssertionError("Independent DST needle roles do not align")
    def hotspots(data,role=None):
        values=[dict(cell_xy_mm=list(cell),value=values.get(role,0) if role else sum(values.values()),by_role=dict(values))
                for cell,values in data.items()]
        return sorted(values,key=lambda row:row["value"],reverse=True)[:10]
    return dict(grid_mm=1.,decoded_dst_sewn_segments=index,
                maximum_all_role_sewn_length_mm_per_mm2=max((sum(v.values()) for v in lengths.values()),default=0),
                maximum_top_fill_length_mm_per_mm2=max((v.get("TopFill",0) for v in lengths.values()),default=0),
                maximum_all_role_punctures_per_mm2=max((sum(v.values()) for v in punctures.values()),default=0),
                maximum_top_fill_punctures_per_mm2=max((v.get("TopFill",0) for v in punctures.values()),default=0),
                sewn_length_hotspots=hotspots(lengths),top_only_length_hotspots=hotspots(lengths,"TopFill"),
                puncture_hotspots=hotspots(punctures),
                method="PyEmbroidery decoded coordinates; entire sewn segments split analytically at 1mm grid crossings; separate TopFill/Underlay/Travel/Lock roles. No fabric acceptance threshold.")


def main():
    root=q.DEFAULT.parent
    rows=[]
    for group in ("comparison","comparison-unpinned","comparison-paired"):
        cases=json.loads((root/group/"fixtures.json").read_text(encoding="utf-8"))["cases"]
        for case in cases:
            row=dict(case=case["name"],group=group,width_mm=case["width_mm"])
            for label in ("baseline","candidate"):
                folder=root/group/label/case["name"]
                result=measure(folder)
                q.write(folder/"independent-segment-density.json",result)
                row[label]={k:v for k,v in result.items() if not k.endswith("hotspots")}
            rows.append(row)
            print(case["name"],"decoded local density checked",flush=True)
    q.write(root/"independent-segment-density.json",rows)


if __name__=="__main__":
    main()
