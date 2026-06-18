FROM airbyte/python-connector-base:2.0.0

COPY . /airbyte/integration_code
RUN pip install /airbyte/integration_code

RUN useradd --create-home --no-log-init --shell /bin/bash airbyte_user
USER airbyte_user

ENV AIRBYTE_ENTRYPOINT="python /airbyte/integration_code/source_gitlab/source.py"
ENTRYPOINT ["python", "/airbyte/integration_code/source_gitlab/source.py"]
