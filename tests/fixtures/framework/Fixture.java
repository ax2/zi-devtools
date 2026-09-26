public class Fixture {
 static volatile Object sink;
 public static void main(String[] args) throws Exception {
  Object a = new Object(), b = new Object();
  Thread x = new Thread(() -> { synchronized(a) {try {Thread.sleep(150);}catch(Exception ignored){} synchronized(b) {} }}, "fixture-lock-a");
  Thread y = new Thread(() -> { synchronized(b) {try {Thread.sleep(150);}catch(Exception ignored){} synchronized(a) {} }}, "fixture-lock-b");
  x.setDaemon(true); y.setDaemon(true); x.start(); y.start();
  long until = System.currentTimeMillis()+3000;
  while (System.currentTimeMillis()<until) { for(int i=0;i<50;i++) {sink=new byte[1024*1024];} Thread.sleep(20); }
  Thread.sleep(3000);
 }
}
