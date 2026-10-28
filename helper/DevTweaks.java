package tomcatrun;

import org.springframework.beans.BeansException;
import org.springframework.beans.MutablePropertyValues;
import org.springframework.beans.factory.config.BeanDefinition;
import org.springframework.beans.factory.config.BeanDefinitionVisitor;
import org.springframework.beans.factory.config.BeanFactoryPostProcessor;
import org.springframework.beans.factory.config.ConfigurableListableBeanFactory;
import org.springframework.beans.factory.support.BeanDefinitionRegistry;

import java.util.ArrayList;
import java.util.List;

/**
 * tomcatrun 개발 전용 — 실행기로 띄운 톰캣에서만 Spring 빈 정의를 고친다.
 *
 * 이 클래스와 등록 XML 은 실행기 폴더의 overlay 에만 있다. 프로젝트 소스 · IDE · 운영에는 없다.
 * 실행기가 주는 시스템 속성이 없으면 아무것도 하지 않는다 (속성마다 따로 켜고 끈다).
 *
 *   tomcatrun.pool.initialSize=N   DB 연결 풀(BasicDataSource)이 처음 여는 연결 수
 *   tomcatrun.lazy=true            싱글턴 빈을 처음 쓸 때 만든다. 기동에서 안 쓰는 빈 · 매퍼 묶음 해석을 건너뛴다
 *   tomcatrun.noSchedule=true      @Scheduled 예약 작업을 돌리지 않는다
 *   tomcatrun.mapperHotswap=true   RefreshableSqlSessionFactoryBean 의 0.5초 감시를 끈다.
 *                                바뀐 매퍼 하나만 HotswapAgent MyBatis 플러그인이 다시 읽게 하려는 것 (둘이 겹치지 않게)
 *
 * 등록: overlay/spring/context-zz-tomcatrun.xml (루트 컨텍스트)
 *       overlay/egovframework/springmvc/application-servlet.xml (MVC 컨텍스트 — 원본과 경로가 같다)
 *   앱이 두 설정을 classpath* 로 읽으므로 원본과 이 파일이 함께 읽힌다. 하나만 찾는 getResource 는 원본을 먼저 찾는다.
 *   컨트롤러 · 서비스 · 매퍼는 MVC 컨텍스트에서 만들어지므로(component-scan 이 거기 있다) 두 곳 모두에 건다.
 */
public class DevTweaks implements BeanFactoryPostProcessor {

	private static final String DBCP        = "org.apache.commons.dbcp.BasicDataSource";
	private static final String REFRESHABLE = "org.mybatis.spring.RefreshableSqlSessionFactoryBean";
	private static final String SCHEDULED   = "org.springframework.scheduling.annotation.ScheduledAnnotationBeanPostProcessor";

	@Override
	public void postProcessBeanFactory(ConfigurableListableBeanFactory bf) throws BeansException {
		final String pool = prop("tomcatrun.pool.initialSize");
		final boolean lazy = "true".equals(prop("tomcatrun.lazy"));
		final boolean noSchedule = "true".equals(prop("tomcatrun.noSchedule"));
		final boolean mapperHotswap = "true".equals(prop("tomcatrun.mapperHotswap"));
		if (pool == null && !lazy && !noSchedule && !mapperHotswap) {
			return;
		}
		String where = bf.getParentBeanFactory() == null ? "루트" : "MVC";
		List<String> done = new ArrayList<String>();

		// 이름 없는 안쪽 빈(Log4jdbcProxyDataSource 생성자 안의 BasicDataSource)까지 돌며 고친다
		final int[] pools = {0};
		final int[] refreshables = {0};
		BeanDefinitionVisitor visitor = new BeanDefinitionVisitor() {
			@Override
			public void visitBeanDefinition(BeanDefinition bd) {
				String cls = bd.getBeanClassName();
				if (pool != null && DBCP.equals(cls)) {
					MutablePropertyValues pv = bd.getPropertyValues();
					pv.add("initialSize", pool);
					if (pv.contains("minIdle")) {
						pv.add("minIdle", pool);
					}
					pools[0]++;
				}
				if (mapperHotswap && REFRESHABLE.equals(cls)) {
					// interval 이 0 이면 감시 타이머를 걸지 않는다 (RefreshableSqlSessionFactoryBean.resetInterval)
					bd.getPropertyValues().add("interval", "0");
					refreshables[0]++;
				}
				super.visitBeanDefinition(bd);
			}

			@Override
			protected String resolveStringValue(String s) {
				return s;
			}
		};
		for (String name : bf.getBeanDefinitionNames()) {
			visitor.visitBeanDefinition(bf.getBeanDefinition(name));
		}
		if (pools[0] > 0) {
			done.add("DB 풀 처음 연결 " + pool + " (" + pools[0] + "개)");
		}
		if (refreshables[0] > 0) {
			done.add("매퍼 묶음 전체 감시 끔 (" + refreshables[0] + "개)");
		}

		if (noSchedule && bf instanceof BeanDefinitionRegistry) {
			// BeanPostProcessor 는 이 단계 뒤에 등록되므로 지금 정의를 빼면 @Scheduled 를 처리할 주체가 없다
			int removed = 0;
			for (String name : bf.getBeanDefinitionNames()) {
				if (SCHEDULED.equals(bf.getBeanDefinition(name).getBeanClassName())) {
					((BeanDefinitionRegistry) bf).removeBeanDefinition(name);
					removed++;
				}
			}
			if (removed > 0) {
				done.add("예약 작업 멈춤");
			}
		}

		if (lazy) {
			// 스프링 기반 시설(ROLE_INFRASTRUCTURE)은 그대로 둔다. BeanPostProcessor · 핸들러 매핑 · 뷰 리졸버는
			// 스프링이 이름이나 형으로 직접 꺼내므로 게으르게 해도 기동 때 만들어진다.
			// 컨트롤러는 핸들러 매핑이 정의의 클래스만 보고 주소를 등록하고, 첫 요청 때 만든다.
			int n = 0;
			for (String name : bf.getBeanDefinitionNames()) {
				BeanDefinition bd = bf.getBeanDefinition(name);
				if (bd.isSingleton() && !bd.isAbstract() && !bd.isLazyInit() && bd.getRole() != BeanDefinition.ROLE_INFRASTRUCTURE) {
					bd.setLazyInit(true);
					n++;
				}
			}
			done.add("지연 초기화 " + n + "개");
		}
		System.out.println("[tomcatrun] " + where + " 설정: " + (done.isEmpty() ? "바꾼 것 없음" : String.join(" · ", done)));
	}

	private static String prop(String key) {
		String v = System.getProperty(key);
		return v == null || v.trim().isEmpty() ? null : v.trim();
	}
}
