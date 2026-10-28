---
name: tomcatrun
description: 두 톰캣을 띄우고 저장 뒤 반영 결과를 보여 주는 개인용 Windows 개발 도구
colors:
  canvas: "oklch(0.975 0 0)"
  side: "oklch(0.955 0 0)"
  panel: "oklch(1 0 0)"
  panel-raised: "oklch(0.935 0 0)"
  line: "oklch(0.885 0 0)"
  line-soft: "oklch(0.95 0 0)"
  line-strong: "oklch(0.78 0 0)"
  ink: "oklch(0.21 0 0)"
  ink-muted: "oklch(0.42 0 0)"
  ink-faint: "oklch(0.52 0 0)"
  ring: "oklch(0.42 0 0 / 0.55)"
  selection: "oklch(0.6 0 0 / 0.28)"
  brand: "oklch(0.8 0.15 78)"
  brand-ink: "oklch(0.24 0.04 70)"
  brand-text: "oklch(0.5 0.12 62)"
  ok-green: "oklch(0.5 0.13 150)"
  crit-red: "oklch(0.54 0.2 27)"
  crit-red-solid: "oklch(0.54 0.2 27)"
  close-red: "oklch(0.58 0.21 25)"
  canvas-dark: "oklch(0.2 0 0)"
  side-dark: "oklch(0.215 0 0)"
  panel-dark: "oklch(0.235 0 0)"
  panel-raised-dark: "oklch(0.275 0 0)"
  line-dark: "oklch(0.305 0 0)"
  line-soft-dark: "oklch(0.255 0 0)"
  line-strong-dark: "oklch(0.4 0 0)"
  ink-dark: "oklch(0.93 0 0)"
  ink-muted-dark: "oklch(0.77 0 0)"
  ink-faint-dark: "oklch(0.65 0 0)"
  ring-dark: "oklch(0.75 0 0 / 0.5)"
  selection-dark: "oklch(0.6 0 0 / 0.35)"
  brand-dark: "oklch(0.82 0.14 80)"
  brand-ink-dark: "oklch(0.22 0.03 75)"
  brand-text-dark: "oklch(0.84 0.13 82)"
  ok-green-dark: "oklch(0.77 0.14 152)"
  crit-red-dark: "oklch(0.71 0.17 25)"
  crit-red-solid-dark: "oklch(0.56 0.2 27)"
  warn-weak: "color-mix(in oklch, {colors.brand} 14%, transparent)"
  warn-line: "color-mix(in oklch, {colors.brand} 55%, transparent)"
  ok-weak: "color-mix(in oklch, {colors.ok-green} 12%, transparent)"
  crit-weak: "color-mix(in oklch, {colors.crit-red} 12%, transparent)"
  crit-line: "color-mix(in oklch, {colors.crit-red} 30%, transparent)"
typography:
  title:
    fontFamily: "Pretendard Variable, Pretendard, Segoe UI, Malgun Gothic, sans-serif"
    fontSize: "14px"
    fontWeight: 600
    lineHeight: 1.55
  body:
    fontFamily: "Pretendard Variable, Pretendard, Segoe UI, Malgun Gothic, sans-serif"
    fontSize: "13px"
    fontWeight: 400
    lineHeight: 1.55
  label:
    fontFamily: "Pretendard Variable, Pretendard, Segoe UI, Malgun Gothic, sans-serif"
    fontSize: "12.5px"
    fontWeight: 500
  caption:
    fontFamily: "Pretendard Variable, Pretendard, Segoe UI, Malgun Gothic, sans-serif"
    fontSize: "12px"
    fontWeight: 400
  badge:
    fontFamily: "Pretendard Variable, Pretendard, Segoe UI, Malgun Gothic, sans-serif"
    fontSize: "11.5px"
    fontWeight: 500
    lineHeight: "18px"
  mono:
    fontFamily: "Cascadia Code, D2Coding, Consolas, Malgun Gothic, monospace"
    fontSize: "12px"
    fontWeight: 400
    lineHeight: 1.55
    fontFeature: "tnum"
rounded:
  bar: "3px"
  chip: "4px"
  control: "6px"
  menu: "8px"
  pill: "999px"
spacing:
  xs: "4px"
  sm: "6px"
  md: "8px"
  lg: "14px"
  xl: "24px"
components:
  button:
    backgroundColor: "{colors.panel}"
    textColor: "{colors.ink}"
    typography: "{typography.label}"
    rounded: "{rounded.control}"
    padding: "0 10px"
    height: "28px"
  button-hover:
    backgroundColor: "{colors.panel-raised}"
  button-primary:
    backgroundColor: "{colors.ink}"
    textColor: "{colors.panel}"
    rounded: "{rounded.control}"
    padding: "0 10px"
    height: "28px"
  button-warn:
    backgroundColor: "{colors.warn-weak}"
    textColor: "{colors.brand-text}"
    rounded: "{rounded.control}"
    padding: "0 10px"
    height: "28px"
  button-danger:
    backgroundColor: "{colors.crit-weak}"
    textColor: "{colors.crit-red}"
    rounded: "{rounded.control}"
    padding: "0 10px"
    height: "28px"
  button-ghost:
    backgroundColor: "transparent"
    textColor: "{colors.ink-muted}"
    rounded: "{rounded.control}"
    padding: "0 10px"
    height: "28px"
  button-sm:
    typography: "{typography.caption}"
    padding: "0 8px"
    height: "24px"
  input:
    backgroundColor: "{colors.panel}"
    textColor: "{colors.ink}"
    rounded: "{rounded.control}"
    padding: "0 8px"
    height: "28px"
  segmented:
    backgroundColor: "{colors.line-soft}"
    rounded: "{rounded.control}"
    padding: "2px"
  segmented-item-on:
    backgroundColor: "{colors.panel}"
    textColor: "{colors.ink}"
    rounded: "{rounded.chip}"
    padding: "0 10px"
    height: "22px"
  tab:
    textColor: "{colors.ink-muted}"
    padding: "0 12px"
    height: "38px"
  tab-on:
    textColor: "{colors.ink}"
  switch-on:
    backgroundColor: "{colors.ink}"
  kind-chip:
    backgroundColor: "{colors.panel-raised}"
    textColor: "{colors.ink-muted}"
    typography: "{typography.badge}"
    rounded: "{rounded.pill}"
    padding: "0 8px"
    height: "20px"
  kind-chip-warn:
    backgroundColor: "{colors.warn-weak}"
    textColor: "{colors.brand-text}"
  kind-chip-crit:
    backgroundColor: "{colors.crit-weak}"
    textColor: "{colors.crit-red}"
  alert-crit:
    backgroundColor: "{colors.crit-weak}"
    textColor: "{colors.crit-red}"
    padding: "7px 14px 7px 16px"
  alert-warn:
    backgroundColor: "{colors.warn-weak}"
    textColor: "{colors.brand-text}"
    padding: "7px 14px 7px 16px"
  titlebar:
    backgroundColor: "{colors.side}"
    textColor: "{colors.ink}"
    height: "36px"
  statusbar:
    backgroundColor: "{colors.panel}"
    textColor: "{colors.ink-muted}"
    typography: "{typography.caption}"
    height: "28px"
---

# Design System: tomcatrun

2026-10-28 : 다른 개발 도구 docket 과 같은 디자인 체계로 바꿨다. 파랑 강조 · 맑은 고딕 · GitHub 풍 회색을 걷고, docket 의 무채색 토큰 · 흑백 주요 단추 · Pretendard 를 들였다. docket 쪽 토큰이 바뀌면 이 파일과 `app/ui/index.html` 머리의 변수를 함께 맞춘다.

## Overview

**Creative North Star: "평범하게 잘 만든 개발 도구"**

이 화면은 docket 과 같은 갈래의 Windows 개발 도구 모양을 흠 없이 내는 것이 전부다. 주제를 입히지 않는다. 특정 업무 시스템의 느낌도, 재미를 노린 장식도 없다. 무채색 회색 판 몇 단 위에 1px 선으로 영역을 나눈다. 주요 단추는 색이 아니라 흑백 반전으로 띄우고, 호박 · 빨강 · 초록 상태 색만 뜻을 싣는다.

구조는 위에서 아래로 제목 줄, 서버 줄, 문제가 있을 때만 나타나는 알림 줄, 탭, 본문(반영 기록 표 · 로그 · 빌드 · SVN · 상태 · 설정), 상태 표시줄이다. 밀도는 데스크톱 도구답게 촘촘하다. 본문 13px, 단추 높이 28px, 표 줄 간격 5px. 시스템이 어두우면 어두운 판으로 바뀌고 밝으면 밝은 판이다(`prefers-color-scheme`). 사용자의 시스템은 어둡다.

창은 대부분 최소화돼 있다가 문제가 있을 때 열린다. 그래서 정상일 때는 조용하고, 문제가 있을 때만 색이 화면에 올라온다.

**Key Characteristics:**
- 무채색 회색 판(`canvas` · `side` · `panel` · `panel-raised`)과 1px 선 세 단(`line-soft` · `line` · `line-strong`)
- 강조색이 없다. 주요 단추 · 고른 탭 · 켜진 스위치는 본문 글자색(`ink`)으로 띄운다
- 상태 색 셋: 호박(`brand`) · 빨강(`crit-red`) · 초록(`ok-green`)
- 동봉한 Pretendard, 숫자 · 경로 · 시각은 고정폭
- 작은 모서리(4px · 6px), 칩은 알약. 그림자는 떠 있는 것에만
- 밝음 · 어두움 두 테마가 같은 이름의 토큰을 공유

## Colors

색은 oklch 로 적는다. 판과 글자는 채도 0 의 회색이고, 색은 상태에만 쓴다. 어두운 테마는 같은 역할의 `-dark` 토큰으로 바꿔 끼운다. 옅은 바탕과 선(`warn-weak` · `crit-weak` · `ok-weak` · `warn-line` · `crit-line`)은 상태 색을 `color-mix` 로 투명하게 섞어 만들므로 두 테마가 같은 식을 쓴다.

### Primary
- **강조색 없음.** 주요 단추는 `ink` 바탕에 `panel` 글자(흑백 반전)다. 고른 탭 밑줄, 켜진 스위치, 체크박스 `accent-color`, 새 줄 수 배지도 `ink` 다. 색을 쓰면 단추와 「재시작 필요」 표시가 같은 말을 하게 된다.
- **초점 고리** (`ring`): 반투명 회색 2px 외곽선.

### Secondary — 상태 색
- **호박** (`brand` · `brand-text` · `brand-ink` · `warn-weak` · `warn-line`): 사용자가 손댈 것. 재시작 필요 알림 줄 · 단추 · 칩, 의존 jar 경고, 힙 상한에 가까운 메모리 막대, 설정의 재시작 안내 상자.
- **빨강** (`crit-red` · `crit-red-solid` · `crit-weak` · `crit-line`): 컴파일 실패 알림 줄 · 표 줄 · 칩 · 오류 위치 링크, SVN 삭제 · 충돌, 지운 줄. `crit-red-solid` 에 흰 글자는 지금 손대야 하는 것(빌드 탭 오류 수 · SVN 충돌 C)에만 쓴다.
- **초록** (`ok-green` · `ok-weak`): 서버 실행 중 점과 상태 글자, 「기동 완료」 칩, 준비 점검 통과 아이콘, 추가된 줄.

### Neutral
- **바탕** (`canvas`): 창 바탕, 원문 로그 영역 바탕.
- **곁 판** (`side`): 제목 줄. 바탕보다 한 단 어둡다.
- **판** (`panel`): 서버 줄 · 탭 · 도구 줄 · 표 · 상태 표시줄 · 단추 · 입력 · 메뉴.
- **올린 판** (`panel-raised`): 단추 · 표 줄 hover, 고른 목록 줄, 중립 칩 · 배지 바탕, 막대 바탕.
- **선 세 단**: 표 줄 사이와 세그먼트 틀 · 차이 보기 hunk 바탕은 `line-soft`, 영역 경계와 단추 테두리는 `line`, 입력 테두리 · 떠 있는 판 테두리 · 스크롤 막대 손잡이 · 꺼진 스위치는 `line-strong`.
- **글자 셋** (`ink` · `ink-muted` · `ink-faint`): 본문, 보조 정보와 쉬는 탭, 구분점 · 줄 번호 · 멈춘 서버 점 · 입력칸 안내 글.
- **선택** (`selection`): 글자 선택. SVN 목록 · 로그에서 고른 줄은 `panel-raised` 에 왼쪽 2px `ink` 선으로 낸다.
- **기동 단계 막대**: 회색 다섯 단계(oklch 0.86 → 0.38). 단계는 뜻이 아니라 순서라 상태 색을 쓰지 않는다.

### Named Rules
**The Status-Only Color Rule.** 호박 · 빨강 · 초록은 서버와 반영의 상태를 뜻할 때만 쓴다. 꾸밈이나 분류 구분에 쓰지 않는다. 「바로 반영」 · 「클래스 교체」 · SVN 수정(M) 같은 정상 진행은 무채색 칩이고 글자로 가른다.

**The Running-Green Exception.** docket 은 「실행 중」도 무채색으로 두지만, 여기서는 서버 실행 중 점과 글자를 초록으로 남긴다(2026-10-28 사용자 결정). 창을 열자마자 서버가 떠 있는지 색으로 바로 보여야 한다. 기동 · 중지 중은 docket 처럼 무채색(`ink-muted`) 점이 1.4초 주기로 깜빡인다.

**The Solid-Means-Act-Now Rule.** 꽉 찬 빨강(`crit-red-solid`)은 지금 손을 대야 하는 것(오류 수 · 충돌)에만 쓴다. 나머지 상태는 옅은 바탕(`-weak`)에 상태 색 글자와 같은 계열의 옅은 테두리로 낸다.

## Typography

**Display Font:** 없음. 큰 제목을 쓰지 않는다.
**Body Font:** Pretendard Variable (with Pretendard, Segoe UI, Malgun Gothic, sans-serif)
**Label/Mono Font:** Cascadia Code (with D2Coding, Consolas, Malgun Gothic, monospace), `tabular-nums`

**Character:** docket 과 같은 Pretendard 를 `app/ui/fonts/PretendardVariable.woff2` 로 동봉해 `@font-face` 로 싣는다. Tauri 가 `ui` 폴더를 exe 에 묻으므로 밖에서 받지 않는다. 시스템에 Pretendard 가 깔려 있으면 그것을 먼저 쓰고, 둘 다 없으면 Segoe UI · 맑은 고딕으로 내려간다. 글자는 `antialiased` 로 다듬는다.

### Hierarchy
- **Title** (600, 14px): 서버 이름, 상태 · 설정 쪽의 구획 제목.
- **Body** (400, 13px, 1.55): 본문, 탭 글자(500), 메뉴 항목. 제목 줄 이름은 12.5px 600.
- **Label** (500, 12.5px): 단추 · 입력 · 체크박스 글자.
- **Caption** (400, 12px): 포트 · 메모리 · 시각 · 걸린 시간 · 상태 표시줄 · 보조 설명. 대개 `ink-muted`.
- **Badge** (500, 11.5px): 반영 기록의 구분 칩. 표 머리 · SVN 상태 코드 · 탭 수 배지는 600, 11~11.5px.
- **Mono** (400, 12px, 1.55): 원문 로그 줄, 파일 경로, 오류 위치, 차이 보기, 리비전 번호.

### Named Rules
**The Fixed-Width Facts Rule.** 숫자 · 포트 · 시각 · 경로 · 파일 이름 · 리비전은 고정폭 글꼴과 `tabular-nums` 로 쓴다. 사람이 읽는 문장은 Pretendard 로 쓴다.

**The No-Display Rule.** 가장 큰 글자는 14px 이다. 위계는 크기가 아니라 굵기(400 · 500 · 600)와 글자 색(`ink` · `ink-muted` · `ink-faint`)으로 낸다.

## Layout

창 전체가 세로 flex 다. 제목 줄 · 서버 줄 · 알림 줄 · 탭 · 본문 · 상태 표시줄 순서이고, 본문만 늘어나며 그 안에서 스크롤한다(`body` 는 넘치지 않는다).

- **제목 줄:** 높이 36px, `side` 바탕. 왼쪽에 16px 아이콘과 이름(12.5px 600 · `ink`)과 버전(`ink-faint`), 오른쪽 끝에 44px 폭 창 단추 셋(획 1.1 · `ink-muted`, hover 에 `panel-raised` 와 `ink`). 닫기 hover 만 빨강(`close-red`)이다. 창이 포커스를 잃으면 글자가 `ink-faint` 로 흐려진다. 지금 문제는 알림 줄이 맡으므로 제목 줄에 되풀이하지 않는다(작업 표시줄 제목에는 붙는다).
- **서버 줄:** 두 칸 grid(1fr 1fr), 칸 사이 1px 세로선. 칸 안은 왼쪽 이름 · 상태 두 줄, 오른쪽 단추 묶음. 창 폭 1000px 이하에서 한 칸씩 위아래로 쌓는다.
- **알림 줄:** 아이콘 · 글 · 단추의 세 칸 grid. 문제가 있을 때만 생긴다.
- **탭 줄:** 높이 38px. 왼쪽에 작업 탭, 빈 칸을 두고 오른쪽 끝에 「상태」 · 「설정」.
- **도구 줄:** 최소 높이 44px, 안쪽 여백 8px 14px, 오른쪽 끝 묶음은 `margin-left: auto`.
- **반영 기록 표:** 다섯 칸 grid(64px 64px 96px 1fr 64px, 칸 사이 12px), 줄 여백 5px 14px. 「내용」 칸은 `ink-muted` 한 줄로 말줄임한다. 지금 폭에서 넘치는 줄만 앞에 ▸(`ink-faint`)가 붙고 줄 아무 데나 누르면 펼친다(펼치면 `ink`). 컴파일 오류 위치 목록은 접지 않는다.
- **상태 쪽:** 카드 격자(최소 420px 칸, 넓으면 두 칸). 메모리 · 직전 기동 시간이 나란히, 준비 상태(공통 · 서버별 칸)와 폴더는 한 줄 전체. 폭 1400px 까지.
- **설정 쪽:** 폭 880px 까지의 한 기둥. 구획마다 제목(14px) 하나와 `panel` 띠 하나이고, 띠 안은 이름 왼쪽 · 스위치나 선택 오른쪽의 한 줄 목록(최소 높이 38px, 줄 사이 `line-soft`)이다. 부제 · 설명 글은 없다. 설명은 줄의 `title` 에 둔다. 딸린 옵션(매퍼 하나만 → 핫리로드 · 컴파일러 → 저장하면 컴파일)은 부모 바로 아래 38px 들여 쓰고, 부모가 꺼지면 투명도 .5 로 흐린다. 값(디버거 포트)은 줄 오른쪽에 고정폭 `ink-muted` 로 둔다. 저장 감시 제외는 목록(맨 위에 지울 수 없는 「기본」 줄) · 입력칸 · 예시 표(누르면 입력칸에 들어가는 코드 모양 단추)를 위에서 아래로 놓는다. 버전 관리는 서버마다 한 칸이고, 안은 라벨 112px · 입력 나머지의 두 칸 줄이며, 고른 종류에 쓰이지 않는 칸(SVN 업데이트 · Git 받기)은 숨긴다.
- **SVN:** 왼쪽 목록(최소 280px, 40%)과 오른쪽 차이 보기의 두 칸.
- **상태 표시줄:** 높이 28px. 켜진 설정을 묶음(반영 · 기동 · 앱)별 칩으로 놓고 묶음 사이에 1px 세로선. 칩의 체크 아이콘은 `ink-muted` 다(꾸밈에 상태 색을 쓰지 않는다). 핫리로드 · 저장하면 컴파일은 꺼져도 지운 줄로 남는다. 칩을 누르면 설정의 그 칸으로 간다. 메모리 요약은 오른쪽.

간격은 6px(단추 사이) · 8px(도구 줄 항목) · 14px(가로 안쪽 여백의 기본) · 24px(쪽 여백)이 되풀이된다.

## Elevation & Depth

바닥에 붙은 판은 그림자 없이 판 색 차이와 1px 선으로만 나눈다. 떠 있는 것 — 메뉴 · 알림 조각(toast) · 닫기 확인 대화상자 — 만 `line-strong` 1px 테두리와 그림자(`shadow-pop`)를 단다. docket 의 `.pop` 과 같다.

### Shadow Vocabulary
- **떠 있는 판** (`shadow-pop`): 밝음 `0 10px 28px oklch(0 0 0 / .12), 0 1px 3px oklch(0 0 0 / .08)`, 어두움 `0 14px 36px oklch(0 0 0 / .5), 0 1px 3px oklch(0 0 0 / .35)`.
- **선택 테두리** (`box-shadow: inset 0 0 0 1px var(--line)`): 세그먼트에서 고른 항목 하나.
- **고른 줄 표시** (`box-shadow: inset 2px 0 0 var(--text)`): SVN 목록 · 로그에서 고른 줄.

### Named Rules
**The Float-Only Shadow Rule.** 그림자는 떠 있는 판에만 쓴다. 바닥의 띠 · 카드 · 표에는 쓰지 않는다. 흐림 · 그라데이션 · 빛은 어디에도 쓰지 않는다.

## Shapes

작고 고른 모서리다. 단추 · 입력 · 세그먼트 틀 · 안내 상자 · 알림 조각은 6px, 세그먼트 항목 · 태그 · SVN 상태 코드 · 상태 표시줄 단추는 4px, 메뉴 · 대화상자는 8px(안의 항목 5px), 막대는 3px 다. 반영 기록 구분 칩 · 탭 수 배지 · 스위치 · 스크롤 막대 손잡이는 알약 모양, 서버 점은 지름 9px 원이다. 테두리는 언제나 1px 실선이고, 선택된 탭만 2px 밑줄이다. 아이콘은 24 격자의 선 아이콘(획 1.75, 둥근 끝)을 14px 로 그린다.

## Components

### Buttons
docket 의 단추와 같은 결의 Windows 도구 단추다.
- **Shape:** 작은 모서리(6px), 높이 28px, 1px `line` 테두리, 아이콘과 글 사이 6px.
- **Default:** `panel` 바탕에 `ink` 글자. hover 에 `panel-raised`, 누르면 `line-soft`. 비활성은 투명도 .5.
- **Primary:** `ink` 바탕 · 테두리에 `panel` 글자(흑백 반전). hover 에 투명도 .85. 서버 「시작」, 닫기 확인, SVN 로그인 확인.
- **Warn:** 재시작 단추. `warn-weak` 바탕, `warn-line` 테두리, `brand-text` 글자, 600.
- **Danger:** `crit-weak` 바탕, `crit-line` 테두리, `crit-red` 글자.
- **Ghost:** 테두리 · 바탕 없이 `ink-muted` 글자, hover 에 `panel-raised` 와 `ink`.
- **Small:** 높이 24px, 여백 0 8px, 12px.
- **Focus:** 모든 조작 요소에 2px `ring` 외곽선, 1px 띄움.

### Chips
- **구분 칩:** 반영 기록의 「구분」 칸. docket `Chip` 과 같은 20px 알약, 1px 테두리, 여백 0 8px, Badge 글자. 중립은 `panel-raised` 바탕 · `line` 테두리 · `ink-muted` 글자.
- **상태 변형:** 바로 반영 · 클래스 교체는 무채색(`ink` 글자 · `line-strong` 테두리), 기동 완료는 옅은 초록, 재시작 필요는 옅은 호박, 컴파일 실패는 옅은 빨강. 모두 같은 계열의 옅은 테두리를 단다.
- **SVN 상태 코드:** 4px 모서리 칩. M 은 무채색(`ink` 글자), A 초록 · D 와 ! 빨강 옅은 바탕, C(충돌)만 꽉 찬 빨강.

### Cards / Containers
- **Corner Style:** 바닥 띠는 모서리 없음, 상태 카드 · 설정 목록은 6px.
- **Background:** `panel` 이 `canvas` 위에 놓인다.
- **Shadow Strategy:** 없음(Elevation & Depth 참조).
- **Border:** 띠 아래 1px `line`, 표 줄 사이 1px `line-soft`.
- **Internal Padding:** 가로 14px 가 기본.

### Inputs / Fields
- **Style:** 높이 28px, `panel` 바탕, 1px `line-strong` 테두리, 6px 모서리, 여백 0 8px, 12.5px. 안내 글은 `ink-faint`. 찾기 칸은 왼쪽에 14px 돋보기를 두고 왼쪽 여백 28px, 폭 200px.
- **Focus:** 공통 초점 외곽선(2px `ring`).
- **Checkbox:** 14px, `accent-color` 는 `ink`. 글자는 `ink-muted`.
- **Switch:** 30×18px 알약. 꺼지면 `line-strong`, 켜지면 `ink` 바탕이고 손잡이는 `panel`.
- **Segmented:** `line-soft` 틀(2px 여백, 6px 모서리, 1px `line`) 안에 높이 22px 항목. 고른 항목은 `panel` 바탕, `ink` 600, 안쪽 1px 선.

### Navigation
- **Tabs:** 높이 38px 줄에 글자 탭. 쉬는 탭은 `ink-muted` 500, hover 에 `ink`, 고른 탭은 `ink` 600 과 2px `ink` 밑줄(docket TabsList `line`). 수 배지는 18px 알약, 평소 `panel-raised`, 새 줄은 `ink` 바탕에 `panel` 글자, 오류는 꽉 찬 빨강.
- **Status bar:** 높이 28px, 12px `ink-muted`. 안의 단추는 테두리 없이, hover 에 `panel-raised` 와 `ink`.
- **Menu:** `panel` 바탕, 1px `line-strong`, 8px 모서리, 4px 여백, 최소 폭 240px(최대 520px), `shadow-pop`. 항목 hover · 초점은 `panel-raised`. 단추 메뉴(「열기」 · 「무시…」)는 단추 아래 오른쪽 끝에, 우클릭 메뉴는 마우스 자리에 뜨고 모자라면 왼쪽 · 위로 편다. 묶음 사이는 `line-soft` 1px 구분선, 지금 할 수 없는 항목은 `ink-faint` 로 남긴다(숨기지 않는다). 우클릭 메뉴는 이 판 하나만 쓰고 브라우저 기본 메뉴는 입력칸에서만 뜬다.
- **Toast:** `panel` 바탕, 1px `line-strong`, 6px 모서리, `shadow-pop`.

### 서버 칸 (Signature Component)
점 · 이름 · 포트 · 상태 한 줄과 메모리 · 기동 시간 · PID 한 줄, 오른쪽에 시작 · 재시작 · 중지 · 열기 단추. 점은 멈춤 `ink-faint`, 실행 `ok-green`, 기동 · 중지 중 `ink-muted` 가 1.4초 주기로 깜빡인다(움직임 줄이기 설정이면 멈춘다). 상태 글자는 실행이면 `ok-green`, 기동 · 중지 중이면 `ink`, 멈춤이면 `ink-muted` 다. 재시작이 필요하면 재시작 단추가 Warn 변형으로 바뀌고 건수를 단다.

### 알림 줄 (Signature Component)
서버 줄 바로 아래 전체 폭 띠. 컴파일 실패는 `crit-weak`, 재시작 필요는 `warn-weak`. 아이콘은 상태 색, 글은 `ink`, 굵게 쓴 요지 뒤에 고정폭 파일 위치 링크(밑줄), 오른쪽에 조치 단추. 문제가 없으면 띠가 없다.

## Do's and Don'ts

### Do:
- **Do** 새 화면도 `panel` 띠와 1px `line` 경계로 나누고, 가로 안쪽 여백은 14px 로 맞춘다.
- **Do** 상태는 옅은 바탕(`-weak`)에 상태 색 글자로 내고, 지금 조치해야 하는 것만 꽉 찬 빨강으로 낸다.
- **Do** 주요 단추 · 고른 것 · 켜진 것은 `ink` 로 띄운다.
- **Do** 숫자 · 경로 · 시각은 고정폭과 `tabular-nums` 로 쓴다.
- **Do** 새 색을 넣을 때는 docket 토큰에서 고르고, 밝음 · 어두움 두 값을 함께 정해 `prefers-color-scheme` 블록에 짝을 맞춘다.
- **Do** 아이콘은 같은 선 아이콘 묶음(24 격자, 획 1.75)에 SVG 로 더한다.
- **Do** 정상일 때는 알림 줄 · 꽉 찬 상태 색이 화면에 하나도 없게 한다.

### Don't:
- **Don't** 특정 업무 시스템의 주제 표현이나 재미를 노린 장식을 넣지 않는다(사용자 지시).
- **Don't** 파랑이든 보라든 강조색을 들이지 않는다. 상태 색 셋 말고는 무채색이다.
- **Don't** 바닥에 붙은 판에 그림자를 달지 않는다. 흐림 · 그라데이션은 어디에도 쓰지 않는다.
- **Don't** CDN 등 외부 자원을 싣지 않는다. 글꼴은 동봉한 Pretendard 만 쓴다.
- **Don't** 상태 색을 분류 구분이나 꾸밈에 쓰지 않는다.
- **Don't** 14px 보다 큰 제목 글자를 만들지 않는다.
- **Don't** 거의 바꾸지 않는 옵션을 본문에 늘 펼쳐 두지 않는다. 설정 탭에 두고 상태 표시줄에 켜진 것만 요약한다.
- **Don't** 제목 옆 부제나 항목 아래 설명 글을 펼쳐 두지 않는다. 무엇에 딸렸는지 · 어디에 쓰이는지는 자리(들여쓰기 · 묶음 · 보이고 숨기기)로 보이고, 남는 설명은 `title` 에 둔다. 지금 일어난 일과 복구 방법(재시작 안내 · 오류 · 닫기 확인)은 화면에 둔다.
