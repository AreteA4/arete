from dataclasses import dataclass
from typing import Any, Dict, Optional

from hyperstack.types import ViewDef, StackDefinition


@dataclass
class TypedStateView:
    view_def: ViewDef
    client: Any

    def get(self, key: str, parser=None):
        return self.client.subscribe(self.view_def.view, key=key, parser=parser)

    def watch(self, key: str, parser=None):
        return self.client.subscribe(self.view_def.view, key=key, parser=parser)


@dataclass
class TypedListView:
    view_def: ViewDef
    client: Any

    def get(self, parser=None):
        return self.client.subscribe(self.view_def.view, parser=parser)

    def watch(self, parser=None):
        return self.client.subscribe(self.view_def.view, parser=parser)


@dataclass
class TypedViewGroup:
    state: Optional[TypedStateView] = None
    list: Optional[TypedListView] = None


class TypedViews:
    def __init__(self, groups: Dict[str, TypedViewGroup]):
        self._groups = groups

    def __getattr__(self, name: str) -> TypedViewGroup:
        if name in self._groups:
            return self._groups[name]
        raise AttributeError(name)

    def __getitem__(self, name: str) -> TypedViewGroup:
        return self._groups[name]


def create_typed_views(stack: StackDefinition, client: Any) -> TypedViews:
    groups: Dict[str, TypedViewGroup] = {}

    for name, group in stack.views.items():
        typed_group = TypedViewGroup()
        if group.state:
            typed_group.state = TypedStateView(group.state, client)
        if group.list:
            typed_group.list = TypedListView(group.list, client)
        groups[name] = typed_group

    return TypedViews(groups)
