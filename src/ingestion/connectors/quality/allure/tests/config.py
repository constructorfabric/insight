from __future__ import annotations

import json

from connector_tests import ConfigBuilder, HttpMocker, HttpRequest, HttpResponse, load_fixture

ALLURE_URL = "https://allure.example.test"
PROJECTS_URL = f"{ALLURE_URL}/api/project"
LAUNCHES_URL = f"{ALLURE_URL}/api/launch/__search"
TEST_RESULTS_URL = f"{ALLURE_URL}/api/testresult"
TEST_CASE_SEARCH_URL = f"{ALLURE_URL}/api/testcase/__search"
LAUNCH_ERRORS_URL = f"{ALLURE_URL}/api/launch/error"

FROZEN_NOW = "2026-07-01T00:00:00Z"
NOW_MS = 1782864000000
START_MS = 1775088000000
TEST_CASE_START_MS = 946684800000

_AUTH_HEADER = {"Authorization": "Api-Token test-api-token"}


class AllureConfigBuilder(ConfigBuilder):
    def __init__(self) -> None:
        super().__init__()
        self._config.update({"allure_url": ALLURE_URL, "allure_api_token": "test-api-token", "allure_project_ids": [7]})


def api_request(url: str, params: dict) -> HttpRequest:
    return HttpRequest(url, query_params=params, headers=_AUTH_HEADER)


def page(content: list[dict], number: int = 0, last: bool = True) -> HttpResponse:
    body = load_fixture(
        __file__,
        "page.json",
        content=content,
        number=number,
        first=number == 0,
        last=last,
        numberOfElements=len(content),
        empty=not content,
    )

    return HttpResponse(body=json.dumps(body), status_code=200)


def error(status: int, headers: dict | None = None) -> HttpResponse:
    body = load_fixture(__file__, "error.json", status=status)

    return HttpResponse(body=json.dumps(body), status_code=status, headers=headers or {})


def paged(params: dict, page_index: int | None = None, size: int = 100) -> dict:
    query = {**params, "size": str(size), "sort": "id,ASC"}

    if page_index is not None:
        query["page"] = str(page_index)

    return query


def launch_params(project_id: int, start_ms: int = START_MS, page_index: int | None = None) -> dict:
    return paged({"projectId": str(project_id), "rql": f"lastModifiedDate >= {start_ms}"}, page_index)


def result_params(launch_id: int, page_index: int | None = None) -> dict:
    return paged({"launchId": str(launch_id)}, page_index)


def mock_launch_window(
    http_mocker: HttpMocker, project_id: int, response: HttpResponse | list[HttpResponse] | None = None
) -> HttpRequest:
    request = api_request(LAUNCHES_URL, launch_params(project_id))
    http_mocker.get(request, response or page([]))

    return request


def case_search_params(project_id: int, start_ms: int = TEST_CASE_START_MS, page_index: int | None = None) -> dict:
    return paged({"projectId": str(project_id), "rql": f"lastModifiedDate >= {start_ms}"}, page_index)


def case_overview_url(test_case_id: int) -> str:
    return f"{ALLURE_URL}/api/testcase/{test_case_id}/overview"


def custom_fields_url(project_id: int) -> str:
    return f"{ALLURE_URL}/api/project/{project_id}/cf"


def launch_env_url(launch_id: int) -> str:
    return f"{ALLURE_URL}/api/launch/{launch_id}/env"


def mock_projects(http_mocker: HttpMocker, project_ids: tuple[int, ...] = (7, 12)) -> HttpRequest:
    request = api_request(PROJECTS_URL, paged({}))
    projects = [load_fixture(__file__, "project.json", id=project_id) for project_id in project_ids]
    http_mocker.get(request, page(projects))

    return request


def mock_case_search(
    http_mocker: HttpMocker, project_id: int, response: HttpResponse | list[HttpResponse] | None = None
) -> HttpRequest:
    request = api_request(TEST_CASE_SEARCH_URL, case_search_params(project_id))
    http_mocker.get(request, response or page([]))

    return request
