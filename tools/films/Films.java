// The films' tables: the uniforms the original's own path classes give its
// space shaders, over each film's time (The Shooting Star (Demo) 1.5.2,
// mc26.3). The installer runs this against the player's own copy of the
// original's jar; nothing of the original is in this repository.
//
// A uniform that changes is sampled every quarter tick, then more finely
// wherever a straight line between two samples would miss it, then thinned
// back to the samples a line needs. File format (little-endian):
//   "SSFT", u32 2, u32 uniforms; per uniform: u8 name length, name, u16
//   floats, u32 samples (0: one value of that many floats follows), then
//   sample times and per sample its floats.
import java.lang.reflect.*; import java.util.*; import java.io.*; import java.nio.*; import java.nio.file.*;
// Samples one of the original's film uniform functions over film time, adaptively, into a table.
public class Films {
  static Method m; static int np; static double extra;
  static Map<String,float[]> at(float ct) throws Throwable {
    Map<String,float[]> g=new LinkedHashMap<>(); Object[] a=new Object[np+1]; Class<?>[] pt=m.getParameterTypes();
    a[0]=ct; for(int i=1;i<np;i++) a[i]= pt[i]==double.class?(Object)extra:(Object)0f;
    a[np]=(java.util.function.BiConsumer<String,float[]>)(k,v)->g.put(k,v.clone()); m.invoke(null,a); return g; }
  static float[] flat(Map<String,float[]> g, List<String> names){ int n=0; for(String k:names) n+=g.get(k).length; float[] o=new float[n]; int i=0; for(String k:names) for(float v:g.get(k)) o[i++]=v; return o; }
  static boolean ok(float[] a, float[] b, float[] mid){ for(int i=0;i<a.length;i++){ double e=Math.abs(mid[i]-(a[i]+b[i])*0.5); double s=Math.max(Math.abs(a[i]),Math.abs(b[i])); if (e > 6e-4*s + 1e-4) return false; } return true; }
  static void export(String cls, int arity, float t0, float t1, String out, double turn) throws Throwable {
    m=null; np=arity; extra=turn;
    Class<?> c=Class.forName("cyou.rimuru.shootingstardemo."+cls);
    for (Method k:c.getDeclaredMethods()){ Class<?>[] pt=k.getParameterTypes(); if(pt.length==np+1&&pt[np]==java.util.function.BiConsumer.class&&pt[0]==float.class){m=k;} }
    m.setAccessible(true);
    Map<String,float[]> first=at(t0); List<String> names=new ArrayList<>(first.keySet());
    HashMap<Float,Map<String,float[]>> cache=new HashMap<>();
    java.util.function.Function<Float,Map<String,float[]>> ev=(t)->cache.computeIfAbsent(t,(q)->{ try { return at(q); } catch(Throwable e){ throw new RuntimeException(e);} });
    List<Float> grid=new ArrayList<>(); for (float t=t0;t<=t1+1e-4;t+=0.25f) grid.add(t);
    ByteArrayOutputStream bo=new ByteArrayOutputStream(); DataOutputStream o=new DataOutputStream(bo);
    StringBuilder sum=new StringBuilder();
    o.writeBytes("SSFT"); o.writeInt(Integer.reverseBytes(2)); o.writeInt(Integer.reverseBytes(names.size()));
    for(String k:names){ int len=first.get(k).length; boolean v=false; float[] f0=ev.apply(grid.get(0)).get(k);
      for(Float t:grid) if(!Arrays.equals(ev.apply(t).get(k),f0)) { v=true; break; }
      o.writeByte(k.length()); o.writeBytes(k); o.writeShort(Short.reverseBytes((short)len));
      if(!v){ o.writeInt(0); for(float q:f0) o.writeInt(Integer.reverseBytes(Float.floatToIntBits(q))); sum.append(k).append(' '); continue; }
      TreeMap<Float,float[]> s=new TreeMap<>(); for(Float t:grid) s.put(t, ev.apply(t).get(k));
      Deque<float[]> work=new ArrayDeque<>(); Float prev=null; for(Float t:grid){ if(prev!=null) work.add(new float[]{prev,t}); prev=t; }
      while(!work.isEmpty()){ float[] iv=work.poll(); float a=iv[0], b=iv[1], mid=(a+b)*0.5f; if (b-a < 1f/64f+1e-6) continue;
        float[] vm=ev.apply(mid).get(k); if (!ok(s.get(a),s.get(b),vm)) { s.put(mid,vm); work.add(new float[]{a,mid}); work.add(new float[]{mid,b}); } }
      // drop samples a straight line through their neighbours already gives
      List<Float> ts=new ArrayList<>(s.keySet()); List<Float> keep=new ArrayList<>(); keep.add(ts.get(0));
      for(int i=1;i<ts.size()-1;i++){ float a=keep.get(keep.size()-1), b=ts.get(i+1); float[] va=s.get(a), vb=s.get(b), vi=s.get(ts.get(i)); float w=(ts.get(i)-a)/(b-a); boolean fine=true;
        for(int j=0;j<len;j++){ double e=Math.abs(vi[j]-(va[j]+(vb[j]-va[j])*w)); double sc=Math.max(Math.abs(va[j]),Math.abs(vb[j])); if(e>6e-4*sc+1e-4){fine=false;break;} }
        if(!fine) keep.add(ts.get(i)); }
      keep.add(ts.get(ts.size()-1));
      o.writeInt(Integer.reverseBytes(keep.size()));
      for(Float t:keep) o.writeInt(Integer.reverseBytes(Float.floatToIntBits(t)));
      for(Float t:keep) for(float q:s.get(t)) o.writeInt(Integer.reverseBytes(Float.floatToIntBits(q)));
      sum.append(k).append('~').append(keep.size()).append(' ');
    }
    o.flush(); Files.write(Paths.get(out), bo.toByteArray());
    System.out.println(out+": "+bo.size()/1024+" KiB");
  }

  /** Every film table the port reads, into the folder given. */
  public static void main(String[] a) throws Throwable {
    String dir = a.length > 0 ? a[0] : ".";
    Files.createDirectories(Paths.get(dir));
    export("iillllliil", 3, 0, 352, dir + "/star_space.bin", 0);
    export("iillllliil", 2, 230, 330, dir + "/star_gun.bin", 0);
    export("iillllliil", 1, 230, 330, dir + "/star_rays.bin", 0);
    export("illii1li1l", 3, 0, 330, dir + "/gungnir_space.bin", 0);
    export("illii1li1l", 2, 0, 330, dir + "/gungnir_mech.bin", 0);
    for (int q = 0; q < 4; q++) export("ili11l1l1ii", 4, 0, 300, dir + "/stars_space_" + q + ".bin", q * Math.PI / 2);
    export("lliii11iil", 3, 0, 470, dir + "/tess_space.bin", 0);
    export("illili1l111", 3, 0, 540, dir + "/grav_space.bin", 0);
  }
}
