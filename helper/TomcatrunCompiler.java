import javax.tools.Diagnostic;
import javax.tools.DiagnosticCollector;
import javax.tools.JavaCompiler;
import javax.tools.JavaFileObject;
import javax.tools.StandardJavaFileManager;
import javax.tools.ToolProvider;
import java.io.BufferedReader;
import java.io.File;
import java.io.InputStreamReader;
import java.io.PrintStream;
import java.io.PrintWriter;
import java.io.StringWriter;
import java.lang.reflect.Constructor;
import java.lang.reflect.Method;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.HashMap;
import java.util.List;
import java.util.Locale;
import java.util.Map;

/**
 * tomcatrun 상주 컴파일러 — 실행기가 한 번 띄워 두고 저장할 때마다 일을 준다.
 *
 * 컴파일러를 매번 새로 띄우면 JVM 기동 · 컴파일러 적재 · jar 247개 색인을 매번 다시 한다.
 * 띄워 둔 채 다시 쓰면 11,000줄 파일이 javac 프로세스 4.5초 → 0.3~0.6초가 된다 (2026-10-27 실측).
 *
 * 컴파일러는 -Dtomcatrun.compiler=ecj|javac 로 고른다 (기본 ecj).
 *   ecj   : 이클립스 컴파일러의 일괄 진입점(batch Main). 톰캣 lib 의 ecj-*.jar 를 클래스패스에 둔다.
 *           같은 파일이 데워진 뒤 0.3초 · 첫 회 1.0초 (javac 상주는 0.5~0.6초 · 2.4~3.0초).
 *           이클립스가 쓰는 것과 같은 JDT 컴파일러다.
 *           ※ 표준 컴파일러 API(EclipseCompiler)로 부르면 매번 파일 관리자를 다시 세워 0.6~1.5초로 오히려 느렸다. 그래서 일괄 진입점을 쓴다.
 *   javac : JDK 의 tools.jar. ecj-*.jar 를 못 찾으면 이쪽으로 간다. 클래스패스마다 파일 관리자를 하나씩 두고 다시 쓴다.
 *
 * 주고받는 말 (한 줄에 하나 · 탭으로 나눈다 · UTF-8)
 *   받음  COMPILE <출력 폴더> <클래스패스> <인자 파일>     인자 파일에는 소스 경로가 한 줄에 하나
 *   보냄  D <줄>                                            진단 한 줄 (「파일:줄: error: 내용」 꼴과 그 아래 코드 · 위치 표시 줄)
 *         END <성공 true|false> <오류 수> <밀리초>
 */
public class TomcatrunCompiler {

	/** javac — 클래스패스마다 파일 관리자를 하나씩 둔다. jar 색인을 다시 읽지 않는 것이 빨라지는 까닭이다 */
	private static final Map<String, StandardJavaFileManager> MANAGERS = new HashMap<String, StandardJavaFileManager>();
	/** 펼친 클래스패스. jar 가 바뀌면(의존 jar 다시 받기) 실행기가 이 프로세스를 내리고 새로 띄운다 */
	private static final Map<String, String> EXPANDED = new HashMap<String, String>();

	private static JavaCompiler javac;
	/** ECJ 일괄 진입점. 있으면 이쪽을 쓴다. ecj jar 가 없을 때도 이 클래스가 읽히도록 리플렉션으로 잡는다 */
	private static Constructor<?> ecjMain;
	private static Method ecjCompile;

	public static void main(String[] args) throws Exception {
		PrintStream out = new PrintStream(System.out, true, "UTF-8");
		if (!"javac".equals(System.getProperty("tomcatrun.compiler", "ecj"))) {
			try {
				Class<?> c = Class.forName("org.eclipse.jdt.internal.compiler.batch.Main");
				ecjMain = c.getConstructor(PrintWriter.class, PrintWriter.class, boolean.class, Map.class,
						Class.forName("org.eclipse.jdt.core.compiler.CompilationProgress"));
				ecjCompile = c.getMethod("compile", String[].class);
			} catch (ReflectiveOperationException e) {
				ecjMain = null;
			}
		}
		if (ecjMain == null) {
			javac = ToolProvider.getSystemJavaCompiler();
			if (javac == null) {
				out.println("D\t컴파일러가 없습니다. ecj jar 를 클래스패스에 두거나 JDK 의 java 로 띄워야 합니다.");
				out.println("END\tfalse\t1\t0");
				return;
			}
		}
		System.err.println("[tomcatrun] 상주 컴파일러: " + (ecjMain != null ? "ecj" : "javac"));

		BufferedReader in = new BufferedReader(new InputStreamReader(System.in, StandardCharsets.UTF_8));
		String line;
		while ((line = in.readLine()) != null) {
			String[] a = line.split("\t", -1);
			try {
				if ("COMPILE".equals(a[0]) && a.length >= 4) {
					List<File> files = new ArrayList<File>();
					for (String f : Files.readAllLines(new File(a[3]).toPath(), StandardCharsets.UTF_8)) {
						f = f.trim();
						if (f.startsWith("\"") && f.endsWith("\"") && f.length() > 1) {
							f = f.substring(1, f.length() - 1);
						}
						if (!f.isEmpty()) {
							files.add(new File(f));
						}
					}
					long t0 = System.nanoTime();
					int[] result = ecjMain != null ? compileEcj(out, a[1], a[2], files) : compileJavac(out, a[1], a[2], files);
					out.println("END\t" + (result[0] == 1) + "\t" + result[1] + "\t" + (System.nanoTime() - t0) / 1000000L);
				} else {
					out.println("D\t알 수 없는 요청: " + a[0]);
					out.println("END\tfalse\t1\t0");
				}
			} catch (Throwable t) {
				out.println("D\t상주 컴파일러 오류: " + t);
				out.println("END\tfalse\t1\t0");
			}
		}
	}

	/** @return {성공 1|0, 오류 수} */
	private static int[] compileEcj(PrintStream out, String outDir, String cp, List<File> files) throws Exception {
		// -Xemacs : 오류를 「파일:줄: error: 내용」 꼴로 낸다 (실행기 화면이 이 꼴을 눌러 편집기로 연다)
		// -proc:none : 이 프로젝트는 애너테이션 처리기를 쓰지 않는다. jar 마다 처리기를 찾지 않게 한다.
		// -g : Spring 이 @RequestParam 이름을 지역 변수 정보에서 읽는다. 빼면 이름 없는 파라미터가 깨진다.
		List<String> args = new ArrayList<String>(Arrays.asList("-1.8", "-nowarn", "-g", "-encoding", "UTF-8", "-proc:none",
				"-Xemacs", "-d", outDir, "-cp", expand(cp)));
		for (File f : files) {
			args.add(f.getPath());
		}
		StringWriter o = new StringWriter();
		StringWriter e = new StringWriter();
		Object main = ecjMain.newInstance(new PrintWriter(o), new PrintWriter(e), false, null, null);
		boolean ok = (Boolean) ecjCompile.invoke(main, (Object) args.toArray(new String[0]));
		int errors = 0;
		for (String l : (e.toString() + "\n" + o.toString()).split("\r?\n")) {
			// 맨 끝의 「N problems (N errors)」 요약은 실행기가 따로 센다
			if (l.trim().isEmpty() || l.matches("\\d+ problems? \\(.*\\)")) {
				continue;
			}
			if (l.contains(": error: ")) {
				errors++;
			}
			out.println("D\t" + l);
		}
		return new int[] {ok ? 1 : 0, errors};
	}

	/** @return {성공 1|0, 오류 수} */
	private static int[] compileJavac(PrintStream out, String outDir, String cp, List<File> files) {
		StandardJavaFileManager fm = MANAGERS.get(cp);
		if (fm == null) {
			fm = javac.getStandardFileManager(null, Locale.getDefault(), StandardCharsets.UTF_8);
			MANAGERS.put(cp, fm);
		}
		List<String> opts = Arrays.asList("-nowarn", "-g", "-encoding", "UTF-8", "-source", "1.8", "-target", "1.8", "-proc:none",
				"-implicit:none", "-sourcepath", "", "-d", outDir, "-cp", expand(cp));
		DiagnosticCollector<JavaFileObject> dc = new DiagnosticCollector<JavaFileObject>();
		Boolean ok = javac.getTask(null, fm, dc, opts, null, fm.getJavaFileObjectsFromFiles(files)).call();
		int errors = 0;
		for (Diagnostic<? extends JavaFileObject> d : dc.getDiagnostics()) {
			if (d.getKind() != Diagnostic.Kind.ERROR) {
				continue;
			}
			errors++;
			String where = d.getSource() == null ? "" : new File(d.getSource().toUri()).getPath() + ":" + d.getLineNumber() + ": ";
			String[] msg = d.getMessage(Locale.getDefault()).split("\r?\n");
			out.println("D\t" + where + "error: " + msg[0]);
			for (int i = 1; i < msg.length; i++) {
				out.println("D\t    " + msg[i]);
			}
		}
		return new int[] {Boolean.TRUE.equals(ok) ? 1 : 0, errors};
	}

	/** 「폴더/*」 를 jar 목록으로 펼친다. 컴파일러 API 도 ECJ 도 펼치지 않는다 (java 실행기만 펼친다) */
	private static String expand(String cp) {
		String hit = EXPANDED.get(cp);
		if (hit != null) {
			return hit;
		}
		StringBuilder b = new StringBuilder();
		for (String e : cp.split(";")) {
			if (e.endsWith("/*") || e.endsWith("\\*")) {
				File[] jars = new File(e.substring(0, e.length() - 2)).listFiles();
				if (jars != null) {
					Arrays.sort(jars);
					for (File j : jars) {
						if (j.getName().endsWith(".jar")) {
							b.append(j.getPath()).append(File.pathSeparatorChar);
						}
					}
				}
			} else if (!e.isEmpty()) {
				b.append(e).append(File.pathSeparatorChar);
			}
		}
		String s = b.toString();
		EXPANDED.put(cp, s);
		return s;
	}
}
